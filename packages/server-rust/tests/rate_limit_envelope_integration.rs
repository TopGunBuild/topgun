//! Integration test: what the WebSocket read loop charges a connection's rate
//! limiter for a frame, and what a frame the limiter refuses hands to dispatch
//! (TG-RATE-001).
//!
//! The limiter lives inside the read loop and is reachable only through a real
//! socket, so every test here boots `ws_upgrade_handler` behind axum, connects a
//! real WebSocket client and observes the wire.
//!
//! Coverage:
//! - an `OP_BATCH` packed into a `BATCH` envelope drains the bucket like the
//!   same `OP_BATCH` sent on its own, with the top-level frame as the control;
//! - an envelope the bucket cannot cover is answered with one 429 and hands
//!   nothing to dispatch, with an admitted envelope as the positive control.
//!
//! Every test ends the same way: the client sends Close and reads until the
//! stream ends. The server closes its side by a bounded abort of its writer, so
//! the end may arrive as a transport error as well as a clean end of stream —
//! both end the read and neither is a failure.
//!
//! An answer still queued when the server reads a Close is never written: from
//! that point the server side refuses data frames. The tests whose last frame
//! is the one under observation therefore keep reading until the connection has
//! been quiet for a while, and only then send Close; the 429 frames are counted
//! over both phases. A lost or late frame can only lower that count, which
//! turns a test red — it cannot turn one green.

use std::future::{ready, Ready};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use topgun_core::messages::{
    BatchMessage, ClientOp, ClientOpMessage, Message, OpBatchMessage, OpBatchPayload,
};
use topgun_core::{SystemClock, HLC};
use topgun_server::network::config::{ConnectionConfig, NetworkConfig};
use topgun_server::network::handlers::AppState;
use topgun_server::service::config::ServerConfig;
use topgun_server::service::dispatch::{DispatchConfig, PartitionDispatcher};
use topgun_server::service::middleware::build_operation_pipeline;
use topgun_server::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
use topgun_server::service::router::OperationRouter;
use topgun_server::service::OperationService;

const RATE_LIMIT_MESSAGE: &str = "rate limit exceeded, slow down";

/// Upper bound on any single wait for an observation. Its expiry fails the
/// test: nothing is concluded from a wait that ran out.
const GUARD: Duration = Duration::from_secs(10);

type Client =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Records the key of every single write it is handed and answers nothing, so
/// the only frames a connection can receive are the read loop's own.
#[derive(Clone)]
struct RecordingService {
    keys: mpsc::UnboundedSender<String>,
}

impl tower::Service<Operation> for RecordingService {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Ready<Result<OperationResponse, OperationError>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), OperationError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        if let Operation::ClientOp { payload, .. } = &op {
            let _ = self.keys.send(payload.payload.key.clone());
        }
        ready(Ok(OperationResponse::Empty))
    }
}

struct Booted {
    port: u16,
    /// Keys recorded by the service behind the dispatcher; `None` when the
    /// server was booted without one.
    recorded: Option<mpsc::UnboundedReceiver<String>>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

/// Boots the upgrade handler with a bucket of `burst` tokens refilled at
/// `refill` per second.
///
/// Without a recorder the service layer is not wired, so a frame that passes the
/// limiter is dropped silently and the 429 frames are the whole observation.
/// With one, a dispatcher with a single partition worker sits in front of a
/// service that records what reaches it: one worker means one inbox, served in
/// arrival order, which is what lets a test order two writes.
async fn boot(refill: u32, burst: u32, with_recorder: bool) -> Booted {
    let net_config = NetworkConfig {
        connection: ConnectionConfig {
            data_plane_max_ops_per_sec: refill,
            data_plane_ops_burst: burst,
            ..ConnectionConfig::default()
        },
        ..NetworkConfig::default()
    };

    let (recorded, operation_service, dispatcher) = if with_recorder {
        let (keys_tx, keys_rx) = mpsc::unbounded_channel();
        let server_config = ServerConfig::default();
        let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            server_config.node_id.clone(),
            Box::new(SystemClock),
        )));
        let operation_service = Arc::new(OperationService::new(
            hlc,
            Arc::new(ServerConfig::default()),
        ));
        let dispatcher = PartitionDispatcher::new(
            &DispatchConfig {
                worker_count: 1,
                channel_buffer_size: 256,
            },
            move || {
                let mut router = OperationRouter::new();
                router.register(
                    service_names::CRDT,
                    RecordingService {
                        keys: keys_tx.clone(),
                    },
                );
                build_operation_pipeline(router, &ServerConfig::default(), None)
            },
        );
        (
            Some(keys_rx),
            Some(operation_service),
            Some(Arc::new(dispatcher)),
        )
    } else {
        (None, None, None)
    };

    let state = AppState {
        config: Arc::new(net_config),
        operation_service,
        dispatcher,
        ..AppState::for_test()
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind should succeed");
    let port = listener.local_addr().expect("local_addr").port();

    let shutdown_ctrl = Arc::clone(&state.shutdown);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let app = axum::Router::new()
        .route(
            "/ws",
            axum::routing::get(topgun_server::network::handlers::ws_upgrade_handler),
        )
        .with_state(state);

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
                shutdown_ctrl.trigger_shutdown();
            })
            .await
            .expect("serve should not fail");
    });

    Booted {
        port,
        recorded,
        shutdown_tx,
    }
}

async fn connect(port: u16) -> Client {
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (ws, _) = connect_async(&url)
        .await
        .expect("WebSocket connect should succeed");
    ws
}

async fn send(ws: &mut Client, msg: &Message) {
    let bytes = rmp_serde::to_vec_named(msg).expect("message serializes");
    ws.send(WsMessage::Binary(bytes.into()))
        .await
        .expect("frame is sent");
}

/// An `OP_BATCH` of `n` default operations. Nothing validates them: the tests
/// that use it wire no dispatcher.
fn op_batch(n: usize) -> Message {
    Message::OpBatch(OpBatchMessage {
        payload: OpBatchPayload {
            ops: vec![ClientOp::default(); n],
            write_concern: None,
            timeout: None,
        },
    })
}

/// One single write on map `notes`.
fn client_op(key: &str) -> Message {
    Message::ClientOp(ClientOpMessage {
        payload: ClientOp {
            map_name: "notes".to_string(),
            key: key.to_string(),
            ..ClientOp::default()
        },
    })
}

/// A `BATCH` envelope holding `items`, each behind its 4-byte big-endian length
/// prefix. The declared count is the caller's and may lie: the server must not
/// read it.
fn envelope(items: &[Message], declared_count: u32) -> Message {
    let mut data = Vec::new();
    for item in items {
        let body = rmp_serde::to_vec_named(item).expect("inner message serializes");
        let len = u32::try_from(body.len()).expect("inner item fits in u32");
        data.extend_from_slice(&len.to_be_bytes());
        data.extend_from_slice(&body);
    }
    Message::Batch(BatchMessage {
        count: declared_count,
        data,
    })
}

/// Appends the message of `frame` to `messages` if it is an `ERROR` with code
/// 429.
fn collect_429(frame: &WsMessage, messages: &mut Vec<String>) {
    if let WsMessage::Binary(bytes) = frame {
        if let Ok(Message::Error { payload }) = rmp_serde::from_slice::<Message>(bytes) {
            if payload.code == 429 {
                messages.push(payload.message);
            }
        }
    }
}

/// Sends Close, reads the stream to its end and returns the message of every
/// `ERROR` frame with code 429 seen on the way.
async fn close_and_collect_429(ws: Client) -> Vec<String> {
    close_and_collect_429_into(ws, Vec::new()).await
}

async fn close_and_collect_429_into(mut ws: Client, mut messages: Vec<String>) -> Vec<String> {
    ws.send(WsMessage::Close(None))
        .await
        .expect("close frame is sent");

    loop {
        let next = tokio::time::timeout(GUARD, ws.next())
            .await
            .expect("the stream ends within the guard");
        match next {
            // A clean end and a transport error both end the read.
            None | Some(Err(_)) => break,
            Some(Ok(frame)) => collect_429(&frame, &mut messages),
        }
    }
    messages
}

/// How long the connection must stay silent before the client closes it.
const QUIET: Duration = Duration::from_secs(1);

/// Reads until no frame has arrived for [`QUIET`] — every frame restarts the
/// timer — then sends Close and reads the stream to its end. Returns the
/// message of every `ERROR` frame with code 429 seen in either phase.
///
/// The quiet period is what lets the server write the answers it has queued
/// before it reads the Close. If it is too short for a slow machine, an answer
/// is lost and the count comes out lower, never higher.
async fn quiet_then_close_and_collect_429(mut ws: Client) -> Vec<String> {
    let mut messages = Vec::new();
    // The timer running out is the expected way out of this phase, not an
    // observation: nothing is concluded from it but "now send Close".
    while let Ok(next) = tokio::time::timeout(QUIET, ws.next()).await {
        match next {
            // The server ended the stream on its own: there is nothing to close.
            None | Some(Err(_)) => return messages,
            Some(Ok(frame)) => collect_429(&frame, &mut messages),
        }
    }
    close_and_collect_429_into(ws, messages).await
}

/// `(429 count, the distinct messages those frames carry)`: one string when
/// every 429 says the same thing, which is what the assertions compare against.
fn summarize(messages: &[String]) -> (usize, String) {
    let mut distinct: Vec<&str> = messages.iter().map(String::as_str).collect();
    distinct.sort_unstable();
    distinct.dedup();
    (messages.len(), distinct.join(" | "))
}

/// Receives recorded keys until `done` holds for the keys seen so far.
async fn receive_until(
    recorded: &mut mpsc::UnboundedReceiver<String>,
    seen: &mut Vec<String>,
    what: &str,
    done: impl Fn(&[String]) -> bool,
) {
    while !done(seen) {
        let key = tokio::time::timeout(GUARD, recorded.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}; recorded so far: {seen:?}"))
            .expect("the recording service is alive");
        seen.push(key);
    }
}

fn count_prefixed(seen: &[String], prefix: &str) -> usize {
    seen.iter().filter(|k| k.starts_with(prefix)).count()
}

// ---------------------------------------------------------------------------
// What a frame is charged
// ---------------------------------------------------------------------------

/// A bucket of 100 tokens refilled at one per second. The envelope carries one
/// `OP_BATCH` of 100 operations, so it must leave nothing for the two batches of
/// 60 that follow: both are refused. Charged as one item, it would leave 99 and
/// only the second batch of 60 would be refused.
#[tokio::test]
async fn an_enveloped_op_batch_drains_the_bucket_like_a_top_level_one() {
    let booted = boot(1, 100, false).await;
    let mut ws = connect(booted.port).await;

    send(&mut ws, &envelope(&[op_batch(100)], 1)).await;
    send(&mut ws, &op_batch(60)).await;
    send(&mut ws, &op_batch(60)).await;

    let refused = quiet_then_close_and_collect_429(ws).await;
    assert_eq!(
        summarize(&refused),
        (2, RATE_LIMIT_MESSAGE.to_string()),
        "(429 count, message of every 429): an envelope holding one OP_BATCH of 100 operations \
         must drain a 100-token bucket, so both following batches of 60 are refused"
    );

    let _ = booted.shutdown_tx.send(());
}

/// The control: the same 100 operations as a top-level `OP_BATCH` drain the
/// bucket and both following batches are refused.
#[tokio::test]
async fn a_top_level_op_batch_drains_the_bucket() {
    let booted = boot(1, 100, false).await;
    let mut ws = connect(booted.port).await;

    send(&mut ws, &op_batch(100)).await;
    send(&mut ws, &op_batch(60)).await;
    send(&mut ws, &op_batch(60)).await;

    let refused = quiet_then_close_and_collect_429(ws).await;
    assert_eq!(
        summarize(&refused),
        (2, RATE_LIMIT_MESSAGE.to_string()),
        "(429 count, message of every 429): a top-level OP_BATCH of 100 operations drains a \
         100-token bucket, so both following batches of 60 are refused"
    );

    let _ = booted.shutdown_tx.send(());
}

// ---------------------------------------------------------------------------
// What a refused frame hands to dispatch
// ---------------------------------------------------------------------------

/// Drives the two-envelope scenario and returns
/// `(429 count, x- keys at the first read, x- keys by the end, d- keys recorded)`.
///
/// The first envelope (60 writes, keys `d-*`) is admitted under either bucket.
/// The second (70 writes, keys `x-*`) is the one under test, followed by a
/// single top-level write with key `barrier`.
///
/// A count of zero cannot be awaited, so the `x-` count is read at the moment
/// `barrier` is recorded. That is an observation and not an expired wait
/// because of how the pieces are ordered: there is one partition worker, so
/// every write goes to one inbox served in arrival order; a frame moved to a
/// dispatch task is spawned before the read loop reads the next frame, and its
/// task enqueues its first write on its first poll. Under the current-thread
/// runtime these tests run on, the envelope's task is polled before the
/// barrier's, so if anything of the envelope were dispatched its first write
/// would be recorded ahead of `barrier`.
///
/// That ordering is observed, not guaranteed, so the refused case reads the
/// count a second time in a way that does not depend on it: after `barrier` it
/// keeps receiving until the recorder has been silent for [`QUIET`], every key
/// restarting the timer, and counts `x-` over everything seen. Both counts must
/// be zero. A late `x-` key can only turn the test red, never green. The limit:
/// a write dispatched later than the quiet period is still not seen.
///
/// The admitted case awaits all 70 `x-` keys instead, and both counts are read
/// once they have arrived.
async fn two_envelopes_then_a_barrier(
    burst: u32,
    wait_for_every_x: bool,
) -> (usize, usize, usize, usize) {
    let mut booted = boot(1, burst, true).await;
    let mut recorded = booted.recorded.take().expect("recorder is wired");
    let mut ws = connect(booted.port).await;
    let mut seen = Vec::new();

    let d: Vec<Message> = (0..60).map(|i| client_op(&format!("d-{i}"))).collect();
    send(&mut ws, &envelope(&d, 1)).await;
    receive_until(&mut recorded, &mut seen, "60 d- keys", |s| {
        count_prefixed(s, "d-") == 60
    })
    .await;

    let x: Vec<Message> = (0..70).map(|i| client_op(&format!("x-{i}"))).collect();
    send(&mut ws, &envelope(&x, 1)).await;
    send(&mut ws, &client_op("barrier")).await;
    receive_until(&mut recorded, &mut seen, "the barrier key", |s| {
        s.iter().any(|k| k == "barrier")
    })
    .await;
    let x_at_barrier = count_prefixed(&seen, "x-");

    let x_first_read = if wait_for_every_x {
        receive_until(&mut recorded, &mut seen, "70 x- keys", |s| {
            count_prefixed(s, "x-") == 70
        })
        .await;
        count_prefixed(&seen, "x-")
    } else {
        // The timer running out is the expected way out, not an observation:
        // what is concluded comes from the keys received before it did.
        while let Ok(key) = tokio::time::timeout(QUIET, recorded.recv()).await {
            seen.push(key.expect("the recording service is alive"));
        }
        x_at_barrier
    };

    let refused = close_and_collect_429(ws).await;
    let _ = booted.shutdown_tx.send(());

    (
        refused.len(),
        x_first_read,
        count_prefixed(&seen, "x-"),
        count_prefixed(&seen, "d-"),
    )
}

/// 100 tokens: the first envelope takes 60, the second needs 70 against the 40
/// that remain and is refused. One 429, and not one of its 70 writes reaches
/// the service behind the dispatcher: none by the time the barrier is recorded,
/// and none by the end of the quiet period that follows.
#[tokio::test]
async fn a_refused_envelope_hands_nothing_to_dispatch() {
    assert_eq!(
        two_envelopes_then_a_barrier(100, false).await,
        (1, 0, 0, 60),
        "(429 count, x- keys recorded when the barrier arrived, x- keys recorded by the end of \
         the quiet period after it, d- keys recorded): a frame the limiter refuses must hand \
         nothing to dispatch"
    );
}

/// The positive control: under 200 tokens the same second envelope is admitted
/// and every one of its 70 writes is recorded, with no 429.
#[tokio::test]
async fn an_admitted_envelope_hands_every_operation_to_dispatch() {
    assert_eq!(
        two_envelopes_then_a_barrier(200, true).await,
        (0, 70, 70, 60),
        "(429 count, x- keys recorded once all were awaited, x- keys recorded by the end, d- \
         keys recorded): an envelope the bucket covers is dispatched whole"
    );
}
