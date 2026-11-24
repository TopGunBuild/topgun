//! WebSocket upgrade handler with inbound/outbound message loops.
//!
//! Uses the socket-split pattern: the WebSocket is split into a sender
//! (owned by the outbound task) and a receiver (owned by the inbound loop).
//! This avoids holding a single mutable reference across concurrent reads
//! and writes.
//!
//! Authentication is two-phase: Phase 1 reads messages sequentially until
//! the connection is authenticated (or the connection closes). Phase 2
//! spawns each dispatch task concurrently, bounded by a semaphore, so the
//! reader can continue consuming frames while previous dispatches are
//! still in flight. If no JWT secret is configured, Phase 1 is skipped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Version};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::sink::{Sink, SinkExt};
use futures_util::stream::StreamExt;
use hyper::upgrade::{OnUpgrade, Upgraded};
use hyper_util::rt::TokioIo;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::{CloseCode, Data as OpData, OpCode};
use tokio_tungstenite::tungstenite::protocol::frame::Frame;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use topgun_core::hash_to_partition;
use topgun_core::messages::{
    AuthAckData, ClientOp, DeviceAckData, ErrorPayload, Message as TopGunMessage, OpAckMessage,
    OpAckPayload, WriteConcern,
};
use tracing::{debug, warn};

use super::auth::AuthHandler;
use super::decode;
use super::AppState;
use crate::network::config::ConnectionConfig;
use crate::network::connection::{ConnectionHandle, ConnectionId};
use crate::network::device_identity::{frontier_client_id, DeviceIdentityStore};
use crate::network::{ConnectionKind, OutboundMessage};
use crate::service::classify::OperationService;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::operation::{
    CallerOrigin, ClassifyError, ErrorDisposition, OpOutcome, OpVerdict, Operation, OperationError,
    OperationResponse, ALL_ERROR_KINDS,
};
use topgun_core::Principal;

/// Maximum number of in-flight dispatch tasks per connection.
///
/// Each spawned task holds a semaphore permit until its dispatch completes.
/// This bounds memory and task overhead: at 6-11µs per op, 32 slots is
/// ~200µs of parallelism — enough to saturate the pipeline without
/// accumulating an unbounded backlog.
const MAX_IN_FLIGHT: usize = 32;

/// The server side of a client WebSocket connection.
pub(crate) type ServerSocket = WebSocketStream<TokioIo<Upgraded>>;

/// Upgrades an HTTP/1.1 request to a client WebSocket connection, then hands
/// off to `handle_socket` for the message processing loops.
///
/// The upgrade is done here rather than by axum's extractor because axum's
/// socket cannot send continuation frames, and every outbound message larger
/// than one frame needs them (`TG-WS-001`). Its rules are those of axum 0.8's
/// extractor, in the same order and with the same rejection statuses and
/// bodies, so every request gets the answer it got before.
///
/// # Panics
///
/// Never in practice: the accept key is base64, which always converts to a
/// header value.
pub async fn ws_upgrade_handler(State(state): State<AppState>, mut request: Request) -> Response {
    let (key, on_upgrade) = match upgrade_request(&mut request) {
        Ok(parts) => parts,
        Err(rejection) => return rejection.into_response(),
    };
    let chunked = chunk_capability(request.uri().query());
    let config = ws_config(&state.config.connection);
    tokio::spawn(async move {
        let upgraded = match on_upgrade.await {
            Ok(upgraded) => upgraded,
            Err(e) => {
                debug!("WebSocket upgrade failed: {e}");
                return;
            }
        };
        let socket =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config))
                .await;
        handle_socket(socket, state, chunked).await;
    });

    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let headers = response.headers_mut();
    headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    // The accept key is base64, which is always a valid header value, so the
    // conversion cannot fail (axum's own handshake expects it the same way).
    headers.insert(
        header::SEC_WEBSOCKET_ACCEPT,
        HeaderValue::try_from(derive_accept_key(key.as_bytes()))
            .expect("a base64 accept key is a valid header value"),
    );
    response
}

/// A refused upgrade: the status and body axum's extractor answers for it.
type UpgradeRejection = (StatusCode, &'static str);

/// Checks an upgrade request and takes its key and upgrade handle.
///
/// Every rule reads only the first value of its header. An offered
/// `Sec-WebSocket-Protocol` or `Sec-WebSocket-Extensions` is neither refused
/// nor echoed: no subprotocol or extension is negotiated.
fn upgrade_request(request: &mut Request) -> Result<(HeaderValue, OnUpgrade), UpgradeRejection> {
    // Above HTTP/1.1 a WebSocket is opened with CONNECT, which the `get`
    // route this handler is mounted on never passes here.
    if request.version() > Version::HTTP_11 {
        return Err((
            StatusCode::METHOD_NOT_ALLOWED,
            "Request method must be `CONNECT`",
        ));
    }
    if request.method() != Method::GET {
        return Err((
            StatusCode::METHOD_NOT_ALLOWED,
            "Request method must be `GET`",
        ));
    }
    let headers = request.headers();
    let connection_names_upgrade = headers
        .get(header::CONNECTION)
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("upgrade"));
    if !connection_names_upgrade {
        return Err((
            StatusCode::BAD_REQUEST,
            "Connection header did not include 'upgrade'",
        ));
    }
    if !header_is(headers, &header::UPGRADE, "websocket") {
        return Err((
            StatusCode::BAD_REQUEST,
            "`Upgrade` header did not include 'websocket'",
        ));
    }
    let key = headers.get(header::SEC_WEBSOCKET_KEY).cloned().ok_or((
        StatusCode::BAD_REQUEST,
        "`Sec-WebSocket-Key` header missing",
    ))?;
    if !header_is(headers, &header::SEC_WEBSOCKET_VERSION, "13") {
        return Err((
            StatusCode::BAD_REQUEST,
            "`Sec-WebSocket-Version` header did not include '13'",
        ));
    }
    let on_upgrade = request.extensions_mut().remove::<OnUpgrade>().ok_or((
        StatusCode::UPGRADE_REQUIRED,
        "WebSocket request couldn't be upgraded since no upgrade state was present",
    ))?;
    Ok((key, on_upgrade))
}

/// Whether the upgrade request opted in to chunked delivery (`TG-WS-002`):
/// its query holds, as one `&`-separated pair, exactly `tg_chunks=1`.
///
/// The client sends the pair literally, so nothing is percent-decoded, and
/// any other value or spelling leaves the connection on whole messages.
fn chunk_capability(query: Option<&str>) -> bool {
    query.is_some_and(|query| query.split('&').any(|pair| pair == "tg_chunks=1"))
}

/// Whether the first value of `name` equals `value`, ignoring ASCII case.
fn header_is(headers: &HeaderMap, name: &HeaderName, value: &str) -> bool {
    headers
        .get(name)
        .is_some_and(|got| got.as_bytes().eq_ignore_ascii_case(value.as_bytes()))
}

/// The transport settings of a client connection: the connection's four
/// limits, every other setting at tungstenite's default.
fn ws_config(connection: &ConnectionConfig) -> WebSocketConfig {
    WebSocketConfig::default()
        .write_buffer_size(connection.ws_write_buffer_size)
        .max_write_buffer_size(connection.ws_max_write_buffer_size)
        // Cap inbound message/frame size so an unauthenticated client cannot force
        // a large allocation (tungstenite defaults are 64 MiB / 16 MiB) and so the
        // depth-checked decoder only ever sees bounded frames.
        .max_message_size(Some(connection.ws_max_message_size))
        .max_frame_size(Some(connection.ws_max_frame_size))
}

/// Processes a connected WebSocket: registers it, runs message loops, and
/// cleans up on disconnect.
///
/// Two-phase message processing:
/// - Phase 1 (auth): reads messages sequentially until authenticated. If no
///   JWT secret is configured the connection skips directly to Phase 2.
/// - Phase 2 (pipeline): each binary frame spawns a concurrent dispatch task
///   bounded by `MAX_IN_FLIGHT` semaphore permits.
///
/// On exit, the semaphore is closed and drained to ensure all in-flight
/// tasks complete before the connection handle (and its outbound sender)
/// is dropped, allowing the outbound task to flush cleanly.
#[allow(clippy::too_many_lines)]
async fn handle_socket(mut socket: ServerSocket, state: AppState, chunked: bool) {
    let (handle, rx) = state
        .registry
        .register(ConnectionKind::Client, &state.config.connection);
    let conn_id = handle.id;

    debug!("WebSocket connected: {:?}", conn_id);

    // Register the refusal series at zero before this connection can write
    // anything, so a scrape taken between connect and first write reads an
    // explicit zero rather than an absent series. Cost is one `Once` check per
    // connection and nothing per operation.
    register_client_op_refusal_series();

    // Send AUTH_REQUIRED before splitting the socket, so the client
    // knows to authenticate before sending any other messages.
    if let Some(ref secret) = state.jwt_secret {
        let auth_handler = AuthHandler::new(secret.clone(), state.auth_validator.clone());
        if let Err(e) = auth_handler.send_auth_required(&mut socket).await {
            warn!("failed to send AUTH_REQUIRED to {:?}: {}", conn_id, e);
            release_session_state(&state, conn_id);
            state.registry.remove(conn_id);
            return;
        }
    }

    let (sender, mut receiver) = socket.split();

    // Outbound task owns the write half of the socket and drains the mpsc
    // channel in queue order. Its framing is fixed here for the connection's
    // whole lifetime.
    let framing = OutboundFraming::new(&state.config.connection, chunked);
    let outbound_handle = tokio::spawn(outbound_task(sender, rx, framing));

    // Tracks auth state for phase switching. AtomicBool avoids RwLock
    // contention in Phase 2 — set once in Phase 1, read once to decide
    // which phase to enter. handle.metadata is still written during Phase 1
    // so domain services can read the principal.
    let authenticated = AtomicBool::new(false);

    // Semaphore limits in-flight dispatch tasks to MAX_IN_FLIGHT.
    // Closed on shutdown to unblock any pending acquire.
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));

    // Cancellation signal for forced teardown by the reaper. Cloned (cheap —
    // Arc inside) so both read phases can select on it without borrowing the
    // handle across the moves they perform on the disconnect path.
    let cancel = handle.cancel.clone();

    // Phase 1: sequential auth — only proceed when JWT secret is configured.
    // If no secret is set, every connection is pre-authenticated.
    if state.jwt_secret.is_some() {
        // Bound the auth handshake: a client that connects and never finishes
        // authenticating (slowloris) must not hold a connection slot forever.
        // The deadline is enforced per read; malformed frames `continue` the
        // loop but do not reset it.
        let auth_deadline = tokio::time::Instant::now() + state.config.connection.auth_timeout;
        'auth: loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    debug!("connection {:?} reaped during auth phase", conn_id);
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
                () = tokio::time::sleep_until(auth_deadline) => {
                    debug!(
                        "connection {:?} exceeded auth deadline; closing (slowloris guard)",
                        conn_id
                    );
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
                msg = receiver.next() => msg,
            };
            match next {
                Some(Ok(Message::Binary(data))) => {
                    // Depth-checked decode BEFORE auth: our version-independent
                    // guard against an unbounded recursive decode (a deeply-nested
                    // frame), which on a codec without an internal cap would
                    // stack-overflow and abort the whole node from an
                    // unauthenticated client. Over-deep/malformed frames are
                    // dropped, not fatal.
                    let tg_msg = match decode::decode_depth_checked::<TopGunMessage>(&data) {
                        Ok(msg) => msg,
                        Err(e) => {
                            debug!("failed to deserialize message from {:?}: {}", conn_id, e);
                            continue;
                        }
                    };

                    if let TopGunMessage::Auth(ref auth_msg) = tg_msg {
                        if let Some(ref secret) = state.jwt_secret {
                            match AuthHandler::from_state(secret, &state)
                                .handle_auth(
                                    auth_msg,
                                    &handle.tx,
                                    state.config.jwt_clock_skew_secs,
                                    state.config.insecure_forward_auth_errors,
                                )
                                .await
                            {
                                Ok(principal) => {
                                    // Store principal in metadata so domain services can read it.
                                    // AtomicBool is set after metadata write so no reader sees
                                    // authenticated=true without principal being set.
                                    {
                                        let mut meta = handle.metadata.write().await;
                                        meta.authenticated = true;
                                        meta.principal = Some(principal.clone());
                                    }
                                    authenticated.store(true, Ordering::Release);

                                    // Device identity present-or-mint (fail-open — never blocks
                                    // auth). One-shot here is structural: this arm runs once then
                                    // `break 'auth`s, so there is no second JWT-mode bind site.
                                    let (device_id, device_token) = bind_device_identity(
                                        &state,
                                        &handle,
                                        conn_id,
                                        Some(&principal.id),
                                        auth_msg.device_token.as_deref(),
                                    )
                                    .await;

                                    // Send AUTH_ACK with userId + any device identity via the
                                    // outbound channel.
                                    let ack_msg = TopGunMessage::AuthAck(AuthAckData {
                                        user_id: Some(principal.id.clone()),
                                        device_id,
                                        device_token,
                                        ..Default::default()
                                    });
                                    if let Ok(bytes) = rmp_serde::to_vec_named(&ack_msg) {
                                        let _ =
                                            handle.tx.send(OutboundMessage::Binary(bytes)).await;
                                    }

                                    debug!(
                                        user_id = %principal.id,
                                        "connection {:?} authenticated",
                                        conn_id
                                    );

                                    break 'auth;
                                }
                                Err(e) => {
                                    // AUTH_FAIL already sent by handle_auth; close connection
                                    debug!("auth failed for {:?}: {}", conn_id, e);
                                    // Drain semaphore and drop before returning
                                    semaphore.close();
                                    drop(handle);
                                    join_outbound_with_timeout(outbound_handle).await;
                                    release_session_state(&state, conn_id);
                                    state.registry.remove(conn_id);
                                    debug!("WebSocket disconnected: {:?}", conn_id);
                                    return;
                                }
                            }
                        }
                    } else {
                        // Drop non-AUTH messages from unauthenticated connections
                        debug!(
                            "dropping message from unauthenticated connection {:?}",
                            conn_id
                        );
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    debug!("connection {:?} closed during auth phase", conn_id);
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
                Some(Ok(Message::Text(_))) => {
                    warn!(
                        "ignoring text message from connection {:?} -- binary only",
                        conn_id
                    );
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {
                    // Ping/Pong are answered by tungstenite, which never yields a
                    // raw frame on read.
                }
                Some(Err(e)) => {
                    debug!(
                        "WebSocket error on connection {:?} during auth: {}",
                        conn_id, e
                    );
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
            }
        }
    }

    // Resolve principal once for this connection so the authorization middleware
    // can read ctx.principal without performing a registry lookup per operation.
    // This is done after Phase 1 completes so the metadata is guaranteed to be set.
    // Also mark the handshake complete so the reaper switches this connection
    // from the auth-deadline bound to the idle (heartbeat) bound — true for
    // no-auth connections too, which skip Phase 1 entirely.
    let principal: Option<Principal> = {
        let mut meta = handle.metadata.write().await;
        meta.handshake_complete = true;
        meta.principal.clone()
    };

    // Per-connection inbound op-rate limiter (data-plane fairness). Aggregate
    // load shedding (MAX_IN_FLIGHT + worker-inbox Overloaded) bounds total work
    // but not a single abusive peer; this token bucket caps one connection's op
    // rate so a flood is throttled (429 back-off) without starving others or
    // tearing the connection down. Owned solely by this read loop — no locking.
    let mut rate_limiter = crate::network::rate_limit::TokenBucket::new(
        state.config.connection.data_plane_max_ops_per_sec,
        state.config.connection.data_plane_ops_burst,
        std::time::Instant::now(),
    );

    // Phase 2: pipeline mode — each binary frame is dispatched concurrently.
    // The reader continues immediately after spawning, so multiple frames
    // can be in-flight simultaneously up to MAX_IN_FLIGHT.
    loop {
        // Select on the cancel token too, so the reaper can unblock a reader
        // parked on a half-open socket (no FIN, no client traffic). On cancel
        // we fall through to the shared cleanup below.
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                debug!("connection {:?} reaped (idle/half-open)", conn_id);
                break;
            }
            msg = receiver.next() => msg,
        };
        match next {
            Some(Ok(Message::Binary(data))) => {
                let tg_msg = match decode::decode_depth_checked::<TopGunMessage>(&data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        debug!("failed to deserialize message from {:?}: {}", conn_id, e);
                        continue;
                    }
                };

                // Device-credential presentation, handled at the websocket layer (both
                // modes). A token-less client (NO_AUTH, or a JWT client before it has a
                // token) sends DEVICE_HELLO as its first frame instead of an empty-token
                // AUTH — the latter collides with JWT validation and a JWT server would
                // AUTH_FAIL + tear the connection down. DEVICE_HELLO is a distinct,
                // non-AUTH frame: a JWT server drops it in Phase 1 (never reaching here),
                // and here we run present-or-mint and reply DEVICE_ACK. It NEVER enters
                // data-plane dispatch. The identity namespace follows the connection's
                // authenticated principal (Some in the unlikely JWT-in-Phase-2 case, None
                // → sentinel in NO_AUTH).
                if let TopGunMessage::DeviceHello(ref hello) = tg_msg {
                    // One-shot binding (explicit Phase-2 guard): once an identity is bound,
                    // drop any further DEVICE_HELLO so in-flight identity-scoped state is
                    // never silently re-attributed. This also makes the handler a safe
                    // no-op for an already-authenticated JWT connection (bound in Phase 1).
                    // The read loop is sequential, so the first bind wins and later frames
                    // observe device_id = Some.
                    let already_bound = handle.metadata.read().await.device_id.is_some();
                    if already_bound {
                        debug!(
                            "dropping repeat DEVICE_HELLO on bound connection {:?}",
                            conn_id
                        );
                        continue;
                    }
                    let (device_id, device_token) = bind_device_identity(
                        &state,
                        &handle,
                        conn_id,
                        principal.as_ref().map(|p| p.id.as_str()),
                        hello.device_token.as_deref(),
                    )
                    .await;
                    // Reply DEVICE_ACK only when an identity was actually bound (a store
                    // must be wired); network-only test servers stay identity-less.
                    if device_id.is_some() {
                        let ack = TopGunMessage::DeviceAck(DeviceAckData {
                            device_id,
                            device_token,
                        });
                        if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
                            let _ = handle.tx.send(OutboundMessage::Binary(bytes)).await;
                        }
                    }
                    continue;
                }

                // Per-connection inbound op-rate limit. Cost = number of ops the
                // frame carries (a batch counts as its op count) so one peer's
                // flood is throttled fairly. On exceed we send a 429 back-off and
                // drop the frame — the connection stays up and recovers as tokens
                // refill (отбой, не падение).
                //
                // A transport envelope is unpacked here, before the charge, and
                // the unpacked value is what the dispatch task is handed: the
                // cost is computed from the very messages that will be
                // dispatched, so the two cannot come to disagree (TG-RATE-001).
                let frame = match tg_msg {
                    TopGunMessage::Batch(batch) => {
                        InboundFrame::Envelope(UnpackedEnvelope::unpack(&batch.data))
                    }
                    other => InboundFrame::Message(other),
                };
                let op_cost = frame.cost();
                if !rate_limiter.try_consume(op_cost) {
                    debug!(
                        "rate limit exceeded for {:?} (op_cost={}); backing off",
                        conn_id, op_cost
                    );
                    let err_msg = TopGunMessage::Error {
                        payload: ErrorPayload {
                            code: 429,
                            message: "rate limit exceeded, slow down".to_string(),
                            details: None,
                        },
                    };
                    if let Ok(bytes) = rmp_serde::to_vec_named(&err_msg) {
                        // Best-effort, non-blocking: a flooding client is by
                        // definition behind on its outbound channel, so awaiting
                        // here would stall this read loop on the very connection
                        // we are throttling (and would gate its own recovery
                        // tokens). If the channel is full, dropping the 429 is
                        // fine — the client is already getting backpressure.
                        let _ = handle.try_send(OutboundMessage::Binary(bytes));
                    }
                    continue;
                }

                // Confirmed-apply ACK: advance the per-device causal frontier inline.
                // Identity-scoped and connection-ownership-fenced, so it is handled here
                // (it needs the connection's device identity + the ownership registry,
                // both in `state`/`handle`) rather than in the spawned data-plane
                // dispatch task. The read loop is sequential, so cursor monotonicity is
                // naturally preserved across a connection's ACK stream.
                if let InboundFrame::Message(TopGunMessage::ClientApplyAck(ref ack)) = frame {
                    handle_client_apply_ack(
                        &state,
                        &handle,
                        conn_id,
                        principal.as_ref(),
                        ack.cursor,
                    )
                    .await;
                    continue;
                }

                // Acquire a permit before spawning; if the semaphore is closed
                // (shutdown signal), exit the reader loop.
                let Ok(permit) = semaphore.clone().acquire_owned().await else {
                    break; // Semaphore closed — exit reader loop
                };

                let tx = handle.tx.clone();
                let op_service = state.operation_service.clone();
                let dispatcher = state.dispatcher.clone();
                let principal_clone = principal.clone();

                tokio::spawn(async move {
                    dispatch_message(frame, conn_id, principal_clone, op_service, dispatcher, tx)
                        .await;
                    drop(permit); // Release after dispatch completes
                });
            }
            Some(Ok(Message::Close(_))) | None => {
                debug!("close frame received from connection {:?}", conn_id);
                break;
            }
            Some(Ok(Message::Text(_))) => {
                // TopGun uses binary MsgPack only; text messages are unexpected.
                warn!(
                    "ignoring text message from connection {:?} -- binary only",
                    conn_id
                );
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {
                // Ping/Pong are answered by tungstenite, which never yields a
                // raw frame on read.
            }
            Some(Err(e)) => {
                debug!("WebSocket error on connection {:?}: {}", conn_id, e);
                break;
            }
        }
    }

    // Graceful shutdown: acquire all permits to wait for in-flight dispatch
    // tasks to complete. Each task holds one permit and drops it when done.
    // Once all permits are re-acquired, close the semaphore so any racing
    // acquire in the reader loop returns Err (defensive — loop has exited).
    for _ in 0..MAX_IN_FLIGHT {
        let _ = semaphore.acquire().await;
    }
    semaphore.close();

    // All in-flight tasks have completed; drop the handle to close handle.tx.
    // The outbound task will drain remaining buffered messages before exiting.
    drop(handle);

    // Wait for the outbound task to finish flushing, then abort it if it is
    // wedged so a stuck writer cannot linger holding the socket.
    join_outbound_with_timeout(outbound_handle).await;

    release_session_state(&state, conn_id);
    state.registry.remove(conn_id);
    debug!("WebSocket disconnected: {:?}", conn_id);
}

/// Runs device-credential present-or-mint and binds the resulting identity to the
/// connection (one-shot `device_id`) plus connection ownership (TAKEOVER).
///
/// Returns `(device_id, minted_token)` for the `AUTH_ACK`. **Fail-open:** a missing
/// store or any storage error leaves the connection identity-less (`device_id` stays
/// `None`) and NEVER blocks authentication — an attacker can always claim "no token"
/// anyway, so failing an honest user out buys no security.
///
/// `principal_id` is `Some` in JWT mode and `None` in `NO_AUTH` mode (keyed under the
/// frontier sentinel namespace). The caller guarantees the one-shot precondition
/// (JWT: structural single Phase-1 bind; `NO_AUTH`: explicit pre-call guard), so this
/// only sets `device_id` when it is still `None`.
async fn bind_device_identity(
    state: &AppState,
    handle: &Arc<ConnectionHandle>,
    conn_id: ConnectionId,
    principal_id: Option<&str>,
    presented: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(factory) = state.store_factory.as_ref() else {
        return (None, None);
    };
    let dev_store = DeviceIdentityStore::new(factory.data_store());
    // present_or_mint tags the no-auth namespace structurally from the `None` principal;
    // no content sentinel is substituted here (a JWT `sub` can never forge the tag).
    match dev_store.present_or_mint(principal_id, presented).await {
        Ok(binding) => {
            {
                let mut meta = handle.metadata.write().await;
                if meta.device_id.is_none() {
                    meta.device_id = Some(binding.device_id.clone());
                }
            }
            // TAKEOVER: a valid credential for an already-owned identity displaces the
            // prior connection, which is closed so its in-flight identity-scoped
            // actions can be fenced out by `is_current_owner`.
            let identity_key = frontier_client_id(principal_id, &binding.device_id);
            if let Some(displaced) = state
                .registry
                .claim_device_ownership(identity_key.clone(), conn_id)
            {
                if let Some(old) = state.registry.get(displaced) {
                    old.cancel();
                }
                warn!(
                    "device-identity takeover on {:?}: displaced {:?}",
                    conn_id, displaced
                );
            }
            // Rehydrate any persisted confirmed-apply cursor for this KNOWN identity
            // into the in-memory frontier BEFORE any ACK, so a reconnecting device pins
            // the prune low-water-mark at its true cursor instead of falling through the
            // "unknown == forgotten" path (which pins nothing and would let the LWM jump
            // forward). A freshly-minted identity has no persisted cursor and correctly
            // stays untracked (unknown → gated). Best-effort: no store → no-op.
            if let Some(frontier) = state.frontier.as_ref() {
                frontier.rehydrate(&identity_key).await;
            }
            (Some(binding.device_id), binding.minted_token)
        }
        Err(e) => {
            debug!(
                "device present-or-mint failed for {:?}: {} (fail-open, identity-less)",
                conn_id, e
            );
            (None, None)
        }
    }
}

/// Handles a client→server confirmed-apply ACK: advances the per-device causal
/// frontier under the bounded, monotone, connection-ownership-fenced rule.
///
/// The replica identity is derived ENTIRELY from server-authenticated connection
/// state — the authenticated `principal` plus the server-issued `device_id` — never
/// from the wire (the ACK carries only `cursor`). The ACK is accepted ONLY from the
/// connection that currently OWNS the identity (342i `is_current_owner` fencing), so
/// a displaced (taken-over) connection's in-flight stale ACK cannot advance the
/// shared cursor past the current owner's durable state. Identity-less connections
/// and rejected stale ACKs are dropped at `debug` (not errors) — they pin nothing.
async fn handle_client_apply_ack(
    state: &AppState,
    handle: &Arc<ConnectionHandle>,
    conn_id: ConnectionId,
    principal: Option<&Principal>,
    claimed: u64,
) {
    let Some(frontier) = state.frontier.as_ref() else {
        return; // frontier not wired (network-only tests) — ACK is a no-op
    };
    let device_id = { handle.metadata.read().await.device_id.clone() };
    let Some(device_id) = device_id else {
        debug!(
            "dropping confirmed-apply ACK from identity-less {:?}",
            conn_id
        );
        return;
    };
    let client = frontier_client_id(principal.map(|p| p.id.as_str()), &device_id);
    if !state.registry.is_current_owner(&client, conn_id) {
        debug!(
            "rejecting stale confirmed-apply ACK from non-owner {:?}",
            conn_id
        );
        return;
    }
    if frontier.confirm_apply_ack(&client, claimed, conn_id).await {
        debug!(conn = ?conn_id, cursor = claimed, "confirmed-apply cursor advanced");
    }
}

/// Releases all session-scoped registry state for a disconnecting connection.
///
/// Invoked at every exit point in `handle_socket` BEFORE `registry.remove(conn_id)`
/// to ensure lock/topic/counter resources are freed even if the connection
/// closes without explicit release messages from the client.
///
/// Each registry field is `Option<Arc<_>>` — `None` is the in-test default
/// so this function is a no-op in test contexts that do not wire the registries.
/// Order: Lock -> Topic -> Counter -> Query -> Journal -> Search -> Hybrid
/// (mirrors the struct field declaration order).
fn release_session_state(state: &AppState, conn_id: ConnectionId) {
    if let Some(ref reg) = state.lock_registry {
        reg.release_on_disconnect(conn_id);
    }
    if let Some(ref reg) = state.topic_registry {
        reg.release_on_disconnect(conn_id);
    }
    if let Some(ref reg) = state.counter_registry {
        reg.release_on_disconnect(conn_id);
    }
    if let Some(ref reg) = state.query_registry {
        reg.unregister_by_connection(conn_id);
    }
    if let Some(ref reg) = state.journal_store {
        reg.unsubscribe_by_connection(conn_id);
    }
    // Removed-id vectors are intentionally dropped: disconnect cleanup needs no
    // downstream fan-out, only the registry-side removal.
    if let Some(ref reg) = state.search_registry {
        let _ = reg.unregister_by_connection(conn_id);
    }
    if let Some(ref reg) = state.hybrid_search_registry {
        let _ = reg.unregister_by_connection(conn_id);
    }
    // Drop this connection's per-connection `delivered` clamp state; the per-identity
    // cursors are untouched (they survive reconnect via rehydration).
    if let Some(ref frontier) = state.frontier {
        frontier.remove_connection(conn_id);
    }
}

/// Hard upper bound on the number of inner items a single transport `Batch`
/// frame may carry.
///
/// Bounds worst-case per-frame work: the 2 MB inbound frame cap lets a
/// `Batch.data` blob pack roughly 524K minimal 4-byte length-prefixed entries,
/// each of which would otherwise drive a decode + classify + dispatch. Capping
/// at `8_192` keeps per-frame cost bounded while sitting far above any
/// legitimate batch size (tens to low thousands), so honest clients never hit it.
/// The cap is checked on the framing alone, before any inner item is decoded.
const MAX_BATCH_INNER_ITEMS: usize = 8_192;

/// Counts the well-formed length-prefixed inner items packed into a transport
/// `Batch.data` blob, without decoding any inner payload.
///
/// The framing rule is the one [`UnpackedEnvelope::unpack`] walks by: read a
/// 4-byte big-endian length prefix, skip that many bytes, repeat. A truncated
/// trailing prefix or payload ends the walk. Its one caller is that
/// constructor, which uses the count to decide whether the envelope may be
/// decoded at all.
///
/// Comparisons are written against the remaining byte count (`data.len() -
/// offset`) rather than `offset + len` so an adversarial length prefix near
/// `u32::MAX` cannot overflow `usize` on a 32-bit target (where it would
/// otherwise wrap the truncation guard and spin the walk). `offset <= data.len()`
/// holds on every iteration, so the subtraction never underflows.
///
/// Pure pointer arithmetic over the already-in-memory slice: no heap allocation,
/// no inner decode.
fn count_batch_items(data: &[u8]) -> usize {
    let mut offset = 0;
    let mut count = 0;
    while data.len() - offset >= 4 {
        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if len > data.len() - offset {
            // Truncated payload: the unpacker stops here, so do we.
            break;
        }
        offset += len;
        count += 1;
    }
    count
}

/// Cost, in op-rate-limiter tokens, of one message.
///
/// An `OpBatch` costs the number of operations it carries, so a peer cannot
/// lower its cost by packing many operations into one message. Every other
/// message costs one token — a transport `Batch` included: one found inside an
/// envelope is refused at classification and dispatches nothing, and a
/// top-level one is costed by [`UnpackedEnvelope::cost`], not here. Always at
/// least 1, so an empty batch still consumes a token and cannot be used to spin.
fn message_cost(msg: &TopGunMessage) -> u32 {
    let count = match msg {
        TopGunMessage::OpBatch(b) => b.payload.ops.len(),
        _ => 1,
    };
    u32::try_from(count.max(1)).unwrap_or(u32::MAX)
}

/// A transport `BATCH` envelope, unpacked once.
///
/// The read loop builds this value before it charges the connection's rate
/// limiter and then moves it into the dispatch task. The cost presented to the
/// limiter and the messages dispatched therefore come from one decode of one
/// value: there is no second walk over the envelope's bytes that could count a
/// different list than the one dispatched (TG-RATE-001).
enum UnpackedEnvelope {
    /// The envelope frames more than [`MAX_BATCH_INNER_ITEMS`] items. None of
    /// them was decoded; the whole frame is answered with one 413.
    OverCap {
        /// Complete framed items in the envelope.
        framed_items: usize,
    },
    /// The envelope's inner items, decoded.
    Unpacked {
        /// Every inner item that decoded, in envelope order.
        messages: Vec<TopGunMessage>,
        /// Framed items that did not decode. They are skipped by dispatch and
        /// still paid for, so garbage is never cheaper than a message.
        undecodable: usize,
    },
}

impl UnpackedEnvelope {
    /// Unpacks an envelope's `data`: length-prefixed inner messages, each
    /// preceded by a 4-byte big-endian length.
    ///
    /// This is the only place an envelope's framing is walked with a decode and
    /// the only place an inner item is decoded. It is handed the bytes and
    /// nothing else — in particular not the count the envelope declares, which
    /// the peer controls and which therefore cannot enter the cost.
    ///
    /// A truncated trailing prefix or payload ends the walk. An item that does
    /// not decode is skipped and counted.
    fn unpack(data: &[u8]) -> Self {
        // The cap comes first and reads the framing only, so an over-long
        // envelope costs the server no inner decode at all.
        let framed_items = count_batch_items(data);
        if framed_items > MAX_BATCH_INNER_ITEMS {
            return Self::OverCap { framed_items };
        }

        let mut messages = Vec::with_capacity(framed_items);
        let mut undecodable = 0;
        let mut offset = 0;

        // Comparisons use the remaining byte count (`data.len() - offset`)
        // rather than `offset + len` so an adversarial near-`u32::MAX` length
        // cannot overflow `usize` on a 32-bit target; `offset <= data.len()`
        // holds throughout, so the subtraction never underflows.
        while data.len() - offset >= 4 {
            let len = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4;

            if len > data.len() - offset {
                debug!(
                    "truncated batch message (need {} bytes, {} available)",
                    len,
                    data.len() - offset
                );
                break;
            }

            let msg_bytes = &data[offset..offset + len];
            offset += len;

            // Inner messages live inside the batch's opaque `bin` body, which
            // the outer frame's depth pre-scan skips without descending — so
            // each inner message gets its own depth-checked decode, consistent
            // with the top-level Phase 1/2 sites. This is the
            // version-independent guard against an unbounded recursive decode
            // of a deeply-nested inner frame; the pinned rmp_serde also caps
            // recursion at 1024, but that is an unstable codec internal we do
            // not rely on.
            match decode::decode_depth_checked::<TopGunMessage>(msg_bytes) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    debug!("failed to deserialize batch inner message: {}", e);
                    undecodable += 1;
                }
            }
        }

        Self::Unpacked {
            messages,
            undecodable,
        }
    }

    /// Cost, in op-rate-limiter tokens, of the envelope.
    ///
    /// Computed from what the value holds; there is no stored figure a caller
    /// could set. An unpacked envelope costs what its messages would cost sent
    /// on their own, plus one token per item that did not decode, and at least
    /// one token. An over-cap envelope, of which nothing is dispatched, costs
    /// its framed item count.
    fn cost(&self) -> u32 {
        match self {
            Self::OverCap { framed_items } => u32::try_from(*framed_items).unwrap_or(u32::MAX),
            Self::Unpacked {
                messages,
                undecodable,
            } => messages
                .iter()
                .map(message_cost)
                .fold(u32::try_from(*undecodable).unwrap_or(u32::MAX), |sum, c| {
                    sum.saturating_add(c)
                })
                .max(1),
        }
    }
}

/// One inbound frame as the read loop hands it to dispatch: a transport
/// envelope already unpacked, or any other message as decoded.
///
/// The message variant is the large one and by far the common one. It is held
/// inline on purpose: the value is built once per frame and moved straight into
/// the dispatch task, and boxing it would cost every frame a heap allocation to
/// make the rare envelope smaller.
#[allow(clippy::large_enum_variant)]
enum InboundFrame {
    Envelope(UnpackedEnvelope),
    Message(TopGunMessage),
}

impl InboundFrame {
    /// Cost of the frame, in op-rate-limiter tokens: the number of client
    /// operations it carries, whichever way it packs them.
    fn cost(&self) -> u32 {
        match self {
            Self::Envelope(envelope) => envelope.cost(),
            Self::Message(msg) => message_cost(msg),
        }
    }
}

/// Dispatches one deserialized frame through the operation pipeline.
///
/// Takes owned Arc and Sender so this function can be moved into a
/// `tokio::spawn` closure (satisfying the `'static` bound). Helpers called
/// from within this function borrow from its owned locals.
///
/// The inner messages of a `BATCH` envelope are handed on one after another;
/// any other message is handed on as it is. Both go to `dispatch_one_message`,
/// which routes a message and answers it.
async fn dispatch_message(
    frame: InboundFrame,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    operation_service: Option<Arc<OperationService>>,
    dispatcher: Option<Arc<PartitionDispatcher>>,
    tx: mpsc::Sender<OutboundMessage>,
) {
    let (Some(classify_svc), Some(dispatcher)) = (operation_service, dispatcher) else {
        debug!(
            "dispatcher not configured, dropping message from {:?}",
            conn_id
        );
        return;
    };

    match frame {
        InboundFrame::Envelope(envelope) => {
            dispatch_envelope(
                envelope,
                conn_id,
                principal,
                &classify_svc,
                &dispatcher,
                &tx,
            )
            .await;
        }
        InboundFrame::Message(tg_msg) => {
            dispatch_one_message(tg_msg, conn_id, principal, &classify_svc, &dispatcher, &tx).await;
        }
    }
}

/// Routes one message that is not a transport envelope and answers it.
///
/// An operation batch is split by partition; anything else is classified and
/// dispatched as a single operation. Everything a message can be answered with
/// is decided here, so that a caller holding a message has one place to hand it
/// to and the answer cannot depend on which caller that was.
async fn dispatch_one_message(
    tg_msg: TopGunMessage,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    classify_svc: &OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    tx: &mpsc::Sender<OutboundMessage>,
) {
    // Intercept OpBatch messages before generic classify/dispatch.
    // Split by partition so each sub-batch runs on a dedicated partition worker
    // rather than serializing all ops on the single global worker.
    if let TopGunMessage::OpBatch(ref batch_msg) = tg_msg {
        dispatch_op_batch(batch_msg, conn_id, principal, classify_svc, dispatcher, tx).await;
        return;
    }

    // Classify the message into a typed Operation
    match classify_svc.classify(tg_msg, None, CallerOrigin::Client) {
        Ok(mut op) => {
            // Set connection_id for domain services (subscription tracking, heartbeat).
            // Set principal so the authorization middleware can evaluate RBAC without
            // a registry lookup.
            op.set_connection_id(conn_id);
            if let Some(p) = principal.clone() {
                op.set_principal(p);
            }

            // Route through the partition dispatcher (MPSC channel per worker)
            match dispatcher.dispatch(op).await {
                Ok(resp) => {
                    send_operation_response(resp, tx).await;
                }
                Err(OperationError::Overloaded) => {
                    // Worker inbox is full; tell the client to back off and retry.
                    let err_msg = TopGunMessage::Error {
                        payload: ErrorPayload {
                            code: 429,
                            message: "server overloaded, try again later".to_string(),
                            details: None,
                        },
                    };
                    if let Ok(bytes) = rmp_serde::to_vec_named(&err_msg) {
                        let _ = tx.send(OutboundMessage::Binary(bytes)).await;
                    }
                }
                Err(e) => {
                    debug!("dispatch error for {:?}: {}", conn_id, e);
                }
            }
        }
        Err(ClassifyError::TransportEnvelope { variant }) => {
            // A top-level BATCH is unpacked by the caller and never gets here.
            // One nested inside an envelope does, and this is where it is
            // refused: an envelope is one level deep.
            debug!(
                "unexpected transport envelope '{}' from {:?}",
                variant, conn_id
            );
        }
        Err(ClassifyError::AuthMessage { variant }) => {
            // AUTH messages from authenticated connections are unexpected
            debug!(
                "ignoring auth message '{}' from already-authenticated {:?}",
                variant, conn_id
            );
        }
        Err(ClassifyError::ServerToClient { variant }) => {
            // Client should not send server-to-client messages
            debug!(
                "ignoring server-to-client message '{}' from {:?}",
                variant, conn_id
            );
        }
        Err(ClassifyError::RequiresPartitionSplit { variant }) => {
            // An operation batch is split above and never classified whole, so
            // this is a routing bug on the server, not a client mistake: say so
            // loudly and send nothing rather than run the batch off its workers.
            warn!(
                "message '{}' from {:?} reached classification without a partition split",
                variant, conn_id
            );
        }
    }
}

/// Which transport a verdict was produced on.
///
/// An enum rather than a string because it is a closed value set that ends up as
/// a metric label: two transports spelling the same label differently would
/// silently split one series into two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportKind {
    /// Persistent client `WebSocket` connection.
    WebSocket,
    /// One-shot HTTP `/sync` request.
    Http,
}

impl TransportKind {
    /// The metric label for this transport.
    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::WebSocket => "ws",
            Self::Http => "http",
        }
    }
}

/// Everything the verdict fold needs to dispatch a batch and then re-dispatch a
/// single operation under the *same* contract as the batch pass.
///
/// A struct rather than a parameter list because both transports must supply the
/// identical set, and because `write_concern` / `timeout` travelling together
/// with the dispatcher is what stops a singleton being judged under a different
/// contract than the batch attempt that failed.
pub(crate) struct OpBatchDispatchContext<'a> {
    /// Builds the per-partition `Operation::OpBatch` values.
    pub classify_svc: &'a OperationService,
    /// Routes each operation to its partition worker.
    pub dispatcher: &'a Arc<PartitionDispatcher>,
    /// Transport this batch arrived on; becomes the refusal metric's label.
    pub transport: TransportKind,
    /// Caller origin recorded on every dispatched operation.
    pub caller_origin: CallerOrigin,
    /// Client identifier, where the transport knows one.
    pub client_id: Option<String>,
    /// Authenticated principal, used by the authorization middleware.
    pub principal: Option<Principal>,
    /// Connection the batch arrived on, where the transport has one.
    pub connection_id: Option<ConnectionId>,
    /// Write concern of the originating batch. The singleton pass MUST reuse it.
    pub write_concern: Option<WriteConcern>,
    /// Timeout of the originating batch. The singleton pass MUST reuse it.
    pub timeout: Option<u64>,
}

// ---------------------------------------------------------------------------
// Per-operation refusal metric
// ---------------------------------------------------------------------------

/// Counter: permanent per-operation refusals, labelled by `transport` and `reason`.
///
/// Deliberately a different series from `topgun_operation_errors_total`, which
/// counts failed *pipeline calls*: one refused operation raises that counter
/// twice (the batch call plus the singleton re-dispatch that names the operation)
/// and raises this one exactly once. Reading a refusal count off the
/// pipeline-error counter is the confusion this series exists to prevent.
pub(crate) const METRIC_CLIENT_OP_REFUSALS_TOTAL: &str = "topgun_client_op_refusals_total";

/// Guards the one-time registration of the refusal series' whole label space.
static REFUSAL_SERIES_TOUCHED: std::sync::Once = std::sync::Once::new();

/// Registers every `{transport, reason}` refusal series at zero, once per process.
///
/// A counter that has never been incremented does not render on `/metrics` at
/// all, so without this an operator cannot tell "nothing was refused" from "the
/// refusal instrument is missing" — and reading an absent series as a zero is
/// exactly how a broken instrument passes for a healthy server.
///
/// Called from each transport's connection-setup path rather than at boot, so it
/// runs before that transport's first write while still costing one `Once` check
/// per connection and nothing at all per operation.
pub(crate) fn register_client_op_refusal_series() {
    REFUSAL_SERIES_TOUCHED.call_once(touch_client_op_refusal_series);
}

/// Touches every `transport × reason` series once.
///
/// Split from the `Once` guard so a test can drive it under its own recorder: a
/// process-lifetime `Once` fires for whichever recorder happens to be bound
/// first, which would make an assertion about the registration unrepeatable.
///
/// The label space is the full cross-product, not one series per transport: a
/// query for `{transport="ws", reason="forbidden"}` reads a different series from
/// one carrying `transport` alone, and would find it absent.
fn touch_client_op_refusal_series() {
    for transport in [TransportKind::WebSocket, TransportKind::Http] {
        for reason in ALL_ERROR_KINDS {
            // An `increment(0)` is what puts the series in the exporter's
            // registry; merely resolving the handle does not.
            metrics::counter!(
                METRIC_CLIENT_OP_REFUSALS_TOTAL,
                "transport" => transport.as_label(),
                "reason" => reason,
            )
            .increment(0);
        }
    }
}

// ---------------------------------------------------------------------------
// Verdict fold
// ---------------------------------------------------------------------------

impl OpBatchDispatchContext<'_> {
    /// Builds one dispatchable sub-batch operation under this batch's contract.
    ///
    /// Every field the batch pass used is reapplied here, write concern and
    /// timeout included, so a singleton re-dispatch is judged under the same
    /// contract as the batch attempt that failed. A singleton judged under a
    /// different contract could refuse an operation the batch would have accepted
    /// — and the client would retire a write that was never really refused.
    fn sub_batch_operation(
        &self,
        ops: Vec<topgun_core::messages::ClientOp>,
        partition_id: u32,
    ) -> Operation {
        let mut op = self.classify_svc.classify_op_batch_for_partition(
            ops,
            partition_id,
            self.client_id.clone(),
            self.caller_origin,
            self.write_concern.clone(),
            self.timeout,
        );
        if let Some(connection_id) = self.connection_id {
            op.set_connection_id(connection_id);
        }
        if let Some(principal) = self.principal.clone() {
            op.set_principal(principal);
        }
        op
    }
}

/// Precedence of a transient error for the single `ERROR` frame, lowest first.
///
/// `Unauthorized > Overloaded > Timeout > Internal` — auth first because it names
/// the most actionable cause. A total order at all is the point: the alternative
/// is "whichever sub-batch finished last", which makes the error the client sees
/// depend on a task-completion race.
///
/// Exhaustive with no `_` arm, so a new variant has to be placed in the order
/// deliberately.
fn transient_precedence(error: &OperationError) -> u8 {
    match error {
        OperationError::Unauthorized => 0,
        OperationError::Overloaded => 1,
        OperationError::Timeout { .. } => 2,
        OperationError::Internal(_) => 3,
        // Permanent variants never reach the transient sink. Ranking them last
        // keeps the match exhaustive without a catch-all arm.
        OperationError::Forbidden { .. }
        | OperationError::InvalidMapName { .. }
        | OperationError::SchemaInvalid { .. }
        | OperationError::ValueTooLarge { .. }
        | OperationError::WrongService
        | OperationError::UnknownService { .. } => u8::MAX,
    }
}

/// Precedence of a non-attributed permanent error for the single `ERROR` frame.
///
/// `Forbidden > InvalidMapName > SchemaInvalid > ValueTooLarge > WrongService > UnknownService`.
/// The last two are unreachable for an operation batch — the classifier hardcodes
/// the CRDT service name and the server registers it unconditionally — and are
/// kept in the order so the match needs no `_` arm, which is what makes a future
/// variant a compile error here.
fn batch_error_precedence(error: &OperationError) -> u8 {
    match error {
        OperationError::Forbidden { .. } => 0,
        OperationError::InvalidMapName { .. } => 1,
        OperationError::SchemaInvalid { .. } => 2,
        OperationError::ValueTooLarge { .. } => 3,
        OperationError::WrongService => 4,
        OperationError::UnknownService { .. } => 5,
        // Transient variants have their own sink and their own order.
        OperationError::Unauthorized
        | OperationError::Overloaded
        | OperationError::Timeout { .. }
        | OperationError::Internal(_) => u8::MAX,
    }
}

/// Keeps whichever of the held and the candidate error ranks higher.
///
/// Ties keep the error already held, so the outcome does not depend on the order
/// sub-batches happened to complete in.
fn keep_by_precedence(
    slot: &mut Option<OperationError>,
    candidate: OperationError,
    precedence: fn(&OperationError) -> u8,
) {
    let take_candidate = match slot.as_ref() {
        None => true,
        Some(held) => precedence(&candidate) < precedence(held),
    };
    if take_candidate {
        *slot = Some(candidate);
    }
}

/// Whether a permanently failed sub-batch may be re-dispatched one op at a time.
///
/// Only the four admission refusals qualify — `Forbidden`, `InvalidMapName`,
/// `SchemaInvalid` and `ValueTooLarge`. Each is decided before the batch
/// applies anything, so re-dispatching an operation of that sub-batch cannot
/// apply a write twice (TG-SYNC-003). `UnknownService` / `WrongService` describe
/// server misrouting rather than a defect in any one operation, so splitting them
/// would attribute a server fault to a client's write and make the client retire
/// it forever.
///
/// `Unauthorized` is transient and therefore never reaches this gate: a singleton
/// must not retire a valid operation that will succeed once the connection
/// re-authenticates.
///
/// Exhaustive with no `_` arm, so a future error variant must be classified
/// explicitly rather than silently inheriting either answer.
fn is_redispatchable(error: &OperationError) -> bool {
    match error {
        OperationError::Forbidden { .. }
        | OperationError::InvalidMapName { .. }
        | OperationError::SchemaInvalid { .. }
        | OperationError::ValueTooLarge { .. } => true,
        // One arm, two reasons: the misrouting pair above, and the transient
        // variants, which have their own sink and never reach this gate at all.
        OperationError::UnknownService { .. }
        | OperationError::WrongService
        | OperationError::Unauthorized
        | OperationError::Overloaded
        | OperationError::Timeout { .. }
        | OperationError::Internal(_) => false,
    }
}

/// The acknowledgement entry for one accepted operation.
fn accepted_result(op_id: String) -> topgun_core::messages::OpResult {
    topgun_core::messages::OpResult {
        op_id,
        success: true,
        achieved_level: WriteConcern::APPLIED,
        error: None,
    }
}

/// Dispatches one client batch and returns a per-operation verdict for every
/// operation it could attribute one to.
///
/// This is the single implementation of the verdict fold: both transports call
/// it and then only shape frames from its result, so there is no second place
/// where "which write did the server refuse?" is decided.
///
/// **Why a fold at all.** A sub-batch that fails permanently fails as a whole,
/// so the failure names a partition, not a write. Reporting that to the client
/// as a batch error leaves every operation in the batch un-verdicted, and the
/// client keeps re-sending writes the server will never accept.
///
/// **Contract.**
///
/// - `partition_groups` is taken **by value**, so the association between a
///   sub-batch's operations and its result cannot be lost: an operation that was
///   part of an accepted sub-batch is accepted, and one that was part of a
///   refused sub-batch is a candidate for individual attribution.
/// - Optimistic first: dispatch the sub-batches as-is. A sub-batch that succeeds
///   contributes its operations to `OpOutcome::accepted` — but only if some
///   operation of the batch was refused, because that is the only case in which
///   the acknowledgement names operations individually. See the field's own
///   contract before reading an empty `accepted` as "nothing was accepted".
/// - A sub-batch failing with a **transient** error contributes to
///   `OpOutcome::transient`; the batch stays retryable and nothing is retired.
/// - A sub-batch failing **permanently** is re-dispatched one operation at a
///   time, each as its own singleton batch through the same dispatcher, **in the
///   sub-batch's original order**, and each singleton's own outcome becomes that
///   operation's verdict. Only admission refusals are re-dispatched — the ones
///   proven to be decided before anything is applied — so re-dispatching cannot
///   apply a write twice (TG-SYNC-003).
/// - A permanent error that is **not** re-dispatchable, and a sub-batch holding
///   an operation with no id, both land in `OpOutcome::batch_error` and stay a
///   batch-level error. Attribution that cannot be proven is not guessed.
/// - Every refusal in `OpOutcome::refused` names an operation exactly once, and
///   no operation appears in both `accepted` and `refused` (TG-SYNC-001).
/// - The refusal metric is emitted here, once per refusal, so neither transport
///   can forget to count one.
pub(crate) async fn attribute_permanent_failure_per_op(
    partition_groups: Vec<(u32, Vec<topgun_core::messages::ClientOp>)>,
    cx: &OpBatchDispatchContext<'_>,
) -> crate::service::operation::OpOutcome {
    let mut outcome = OpOutcome::default();

    // Optimistic pass: dispatch every sub-batch concurrently, exactly as an
    // unattributed batch does today.
    let mut join_set = tokio::task::JoinSet::new();
    for (partition_id, group_ops) in partition_groups {
        // The classifier consumes the operations it wraps, so the task keeps its
        // own copy of them. Without one the completion carries only a result, and
        // the sub-batch <-> result association — which decides both "these ops
        // were accepted" and "re-dispatch exactly these ops" — is unrecoverable.
        let sub_op = cx.sub_batch_operation(group_ops.clone(), partition_id);
        let dispatcher = Arc::clone(cx.dispatcher);
        join_set.spawn(async move {
            let result = dispatcher.dispatch(sub_op).await;
            (partition_id, group_ops, result)
        });
    }

    let mut accepted_groups: Vec<(u32, Vec<ClientOp>)> = Vec::new();
    let mut permanent_failures: Vec<(u32, Vec<ClientOp>, OperationError)> = Vec::new();

    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok((partition_id, ops, Ok(_resp))) => accepted_groups.push((partition_id, ops)),
            Ok((partition_id, ops, Err(e))) => match e.disposition() {
                ErrorDisposition::Transient => {
                    keep_by_precedence(&mut outcome.transient, e, transient_precedence);
                }
                ErrorDisposition::Permanent => permanent_failures.push((partition_id, ops, e)),
            },
            Err(join_err) => {
                // A panicked or cancelled worker task names no operation, and
                // `Internal` is transient, so the batch stays retryable.
                keep_by_precedence(
                    &mut outcome.transient,
                    OperationError::Internal(anyhow::anyhow!("join error: {join_err}")),
                    transient_precedence,
                );
            }
        }
    }

    // Sub-batches complete in whatever order the runtime finishes them, so the
    // fallback is walked in partition order instead: which frames a client
    // receives for one batch must not depend on a task-completion race.
    permanent_failures.sort_by_key(|(partition_id, _, _)| *partition_id);

    let mut singleton_accepted: Vec<String> = Vec::new();
    for (partition_id, ops, error) in permanent_failures {
        if !is_redispatchable(&error) {
            keep_by_precedence(&mut outcome.batch_error, error, batch_error_precedence);
            continue;
        }
        // An operation with no id cannot be named in a rejection, and guessing
        // which write was refused is worse than reporting that the batch failed.
        // Collecting into `Option<Vec<_>>` yields `None` if ANY op lacks an id.
        let Some(op_ids) = ops
            .iter()
            .map(|op| op.id.clone())
            .collect::<Option<Vec<String>>>()
        else {
            keep_by_precedence(&mut outcome.batch_error, error, batch_error_precedence);
            continue;
        };

        // Sequentially, in the sub-batch's original order: apply order is
        // observable through the Event Journal, so it is part of what the caller
        // sees and not an implementation detail.
        for (op, op_id) in ops.into_iter().zip(op_ids) {
            let singleton = cx.sub_batch_operation(vec![op], partition_id);
            match cx.dispatcher.dispatch(singleton).await {
                Ok(_) => singleton_accepted.push(op_id),
                Err(e) => match e.disposition() {
                    ErrorDisposition::Permanent => {
                        metrics::counter!(
                            METRIC_CLIENT_OP_REFUSALS_TOTAL,
                            "transport" => cx.transport.as_label(),
                            "reason" => e.error_kind(),
                        )
                        .increment(1);
                        outcome.refused.push(OpVerdict::Refused {
                            op_id,
                            code: e.wire_code(),
                            reason: format!("{e}"),
                            kind: e.error_kind(),
                        });
                    }
                    ErrorDisposition::Transient => {
                        keep_by_precedence(&mut outcome.transient, e, transient_precedence);
                    }
                },
            }
        }
    }

    // The acknowledgement names individual operations only when some operation
    // was refused: a batch nobody refused anything in is acknowledged by its last
    // id with no `results` at all, so materializing the per-operation vector
    // there would be hot-path cost for something no caller reads.
    if !outcome.refused.is_empty() {
        accepted_groups.sort_by_key(|(partition_id, _)| *partition_id);
        outcome.accepted = accepted_groups
            .into_iter()
            .flat_map(|(_, ops)| ops)
            .filter_map(|op| op.id)
            .chain(singleton_accepted)
            .map(accepted_result)
            .collect();
    }

    outcome
}

/// The `lastId` an acknowledgement that names operations individually may carry.
///
/// The numeric maximum of the accepted ids, returned in its original spelling.
/// Numeric and not lexicographic: operation ids are stringified autoincrement
/// integers, so a lexicographic maximum picks `"9"` over `"10"` and would leave
/// the client believing everything up to `"9"` was acknowledged.
///
/// `None` when no accepted id is a number at all. A non-numeric `lastId` makes
/// the client mark **every** pending operation as synced, which on an exchange
/// that refused something would durably discard a write the server never
/// accepted — so the caller sends no acknowledgement instead, and the accepted
/// operations are re-sent on the next flush.
pub(crate) fn numeric_max_op_id(accepted: &[topgun_core::messages::OpResult]) -> Option<String> {
    accepted
        .iter()
        .filter_map(|result| result.op_id.parse::<u64>().ok().map(|n| (n, &result.op_id)))
        .max_by_key(|(n, _)| *n)
        .map(|(_, op_id)| op_id.clone())
}

/// Serializes one frame and queues it on the connection's outbound channel.
///
/// `to_vec_named` and not `to_vec`: the client decodes `MsgPack` maps by field
/// name, so an array-encoded frame is unreadable to it.
async fn send_frame(msg: &TopGunMessage, tx: &mpsc::Sender<OutboundMessage>) {
    if let Ok(bytes) = rmp_serde::to_vec_named(msg) {
        let _ = tx.send(OutboundMessage::Binary(bytes)).await;
    }
}

/// Queues one `OP_REJECTED` frame per attributed refusal.
///
/// Called before anything else the exchange sends, and unconditionally: an
/// exchange that also hit a transient error still reports the operations it
/// already reached a verdict on. The outbound channel is FIFO, so queueing these
/// first IS the mechanism behind "a refusal always reaches the client ahead of
/// any acknowledgement" (TG-SYNC-002) — there is no second ordering guard on the
/// server side.
///
/// `permanent` is `true` on every frame this sends: the refusal sink holds only
/// verdicts a retry cannot change, and a client that read a terminal refusal as
/// retryable would re-send the write forever.
async fn send_op_rejected_frames(refused: &[OpVerdict], tx: &mpsc::Sender<OutboundMessage>) {
    for verdict in refused {
        let OpVerdict::Refused {
            op_id,
            code,
            reason,
            ..
        } = verdict;
        let rejected = TopGunMessage::OpRejected(topgun_core::messages::OpRejectedMessage {
            payload: topgun_core::messages::OpRejectedPayload {
                op_id: op_id.clone(),
                reason: reason.clone(),
                code: Some(*code),
                permanent: true,
            },
        });
        send_frame(&rejected, tx).await;
    }
}

/// Splits an `OpBatch` by partition, dispatches all sub-batches concurrently and
/// answers with a per-operation verdict for everything the batch touched.
///
/// Groups the batch's ops by `hash_to_partition(key)`, hands the groups to the
/// shared verdict fold — which dispatches one `Operation::OpBatch` per group and,
/// when a group fails permanently, re-dispatches its operations one at a time to
/// find out which of them the server actually refused — and then only shapes
/// frames from the fold's outcome.
///
/// **Frames, in this order.** Every refusal becomes one `OP_REJECTED`, and all of
/// them precede any acknowledgement (TG-SYNC-002); the outbound channel is FIFO,
/// so queueing them first is the ordering mechanism. Then exactly one of:
///
/// - a transient (or non-attributed permanent) error → one `ERROR` frame and **no
///   acknowledgement**, even when some operations were accepted;
/// - nothing refused → today's `OP_ACK { lastId, achievedLevel }` with no
///   `results`, byte-identical to the acknowledgement a server without per-operation
///   verdicts sends;
/// - everything refused → **nothing further**: the last `OP_REJECTED` terminates
///   the exchange, because a batch in which every operation was refused has
///   nothing to acknowledge;
/// - otherwise → one `OP_ACK` naming the accepted operations in `results`.
///
/// **Why no partial acknowledgement beside a transient error.** Take sub-batches
/// `{1,3,5}` (all refused) and `{2,4,6}` (transient). An acknowledgement whose
/// `lastId` is the largest accepted id lets the client durably delete every
/// operation at or below it — including 2 and 4, which nobody ever accepted. The
/// operations a client re-sends must be exactly the operations nobody confirmed.
async fn dispatch_op_batch(
    batch_msg: &topgun_core::messages::OpBatchMessage,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    classify_svc: &OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    tx: &mpsc::Sender<OutboundMessage>,
) {
    let ops = &batch_msg.payload.ops;

    if ops.is_empty() {
        // No frame at all: an empty batch has no operation to acknowledge, and an
        // acknowledgement that names no operation id is read by a client as
        // "every pending write was applied" and durably retires writes the server
        // never saw (TG-SYNC-004).
        return;
    }

    // Compute lastId from the last op in the original batch order.
    //
    // The fallback is the id-less residue: only a client that sends operations
    // without ids can reach it, and such a client has no other acknowledgement
    // to wait for. A batch that carries ids is always answered with one of them.
    let last_id = ops
        .last()
        .and_then(|op| op.id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Group ops by their partition so each group targets one partition worker.
    let mut partition_groups: HashMap<u32, Vec<topgun_core::messages::ClientOp>> = HashMap::new();
    for op in ops {
        let partition_id = hash_to_partition(&op.key);
        partition_groups
            .entry(partition_id)
            .or_default()
            .push(op.clone());
    }

    let cx = OpBatchDispatchContext {
        classify_svc,
        dispatcher,
        transport: TransportKind::WebSocket,
        caller_origin: CallerOrigin::Client,
        client_id: None,
        principal,
        connection_id: Some(conn_id),
        write_concern: batch_msg.payload.write_concern.clone(),
        timeout: batch_msg.payload.timeout,
    };

    let mut outcome =
        attribute_permanent_failure_per_op(partition_groups.into_iter().collect(), &cx).await;

    send_op_rejected_frames(&outcome.refused, tx).await;

    // (i) A transient error outranks a non-attributed permanent one: retrying is
    //     the only response that can still change the outcome.
    if let Some(err) = outcome
        .transient
        .take()
        .or_else(|| outcome.batch_error.take())
    {
        debug!("dispatch_op_batch error for {:?}: {}", conn_id, err);
        let (code, message) = match err {
            OperationError::Overloaded => (429, "server overloaded, try again later".to_string()),
            ref e => (e.wire_code(), format!("{e}")),
        };
        let err_response = TopGunMessage::Error {
            payload: ErrorPayload {
                code,
                message,
                details: None,
            },
        };
        send_frame(&err_response, tx).await;
        return;
    }

    // (ii) Nothing refused — today's aggregated acknowledgement, unchanged.
    //      `achievedLevel` is set explicitly because each sub-batch's CRDT merge
    //      succeeded in memory, and the per-sub-batch acks were folded away.
    //
    //      This branch MUST stay ahead of the one below: the fold populates
    //      `accepted` only once something was refused, so an empty `accepted`
    //      here means "not computed", not "nothing was accepted".
    if outcome.refused.is_empty() {
        let ack = TopGunMessage::OpAck(OpAckMessage {
            payload: OpAckPayload {
                last_id,
                achieved_level: Some(WriteConcern::APPLIED),
                ..Default::default()
            },
        });
        send_frame(&ack, tx).await;
        return;
    }

    // (iii) Every id-bearing operation was refused: the `OP_REJECTED` frames are
    //       the whole answer. No `OP_ACK` (there is nothing to acknowledge) and
    //       no `ERROR` (the exchange is already explained operation by operation).
    if outcome.accepted.is_empty() {
        return;
    }

    // Same silence for an acceptance set no numeric `lastId` can address: see
    // `numeric_max_op_id`. The accepted operations stay pending and are re-sent,
    // which is safe; falsely acknowledging the refused one would not be.
    let Some(partial_last_id) = numeric_max_op_id(&outcome.accepted) else {
        warn!(
            "op batch for {:?}: accepted operations carry no numeric id; \
             sending no partial ack rather than one the client would over-apply",
            conn_id
        );
        return;
    };

    // (iv) Partial: `results` is the authoritative acceptance set, and it is
    //      never empty on this path.
    let ack = TopGunMessage::OpAck(OpAckMessage {
        payload: OpAckPayload {
            last_id: partial_last_id,
            achieved_level: Some(WriteConcern::APPLIED),
            results: Some(std::mem::take(&mut outcome.accepted)),
        },
    });
    send_frame(&ack, tx).await;
}

/// Dispatches the inner messages of an unpacked `BATCH` envelope, one at a time.
///
/// Takes the value the read loop built and charged for and reads no envelope
/// bytes of its own: nothing here walks a framing or decodes an item.
async fn dispatch_envelope(
    envelope: UnpackedEnvelope,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    classify_svc: &OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    tx: &mpsc::Sender<OutboundMessage>,
) {
    let messages = match envelope {
        // Drop the WHOLE frame if it packs more inner items than the cap. This
        // matches the whole-frame token-exhaustion behavior: nothing of an
        // over-long envelope is dispatched, so the client gets one answer for
        // the frame instead of answers for a part of it that it cannot
        // identify. The client is told via an explicit error rather than a
        // silent drop.
        UnpackedEnvelope::OverCap { framed_items } => {
            debug!(
                "batch from {:?} exceeds max inner items ({} > {}); dropping whole frame",
                conn_id, framed_items, MAX_BATCH_INNER_ITEMS
            );
            let err = TopGunMessage::Error {
                payload: ErrorPayload {
                    code: 413,
                    message: "batch exceeds maximum inner item count".to_string(),
                    details: None,
                },
            };
            if let Ok(bytes) = rmp_serde::to_vec_named(&err) {
                let _ = tx.send(OutboundMessage::Binary(bytes)).await;
            }
            return;
        }
        UnpackedEnvelope::Unpacked { messages, .. } => messages,
    };

    for inner_msg in messages {
        // One inner message at a time, awaited: that is what keeps the answers
        // in envelope order. Handing it to the entry a top-level message takes
        // is what makes the answer the same on both paths (TG-DISP-001).
        dispatch_one_message(
            inner_msg,
            conn_id,
            principal.clone(),
            classify_svc,
            dispatcher,
            tx,
        )
        .await;
    }
}

/// Sends an `OperationResponse` as outbound WebSocket message(s).
///
/// Exactly three variants put a frame on the wire:
/// - `Message` -> serialize and send as binary frame
/// - `Messages` -> serialize each individually and send as separate frames
/// - `NotImplemented` -> construct `Error` with code 501
///
/// Every other variant sends nothing. In particular this function never builds
/// an operation acknowledgement: that frame is reserved for the answer to an
/// operation batch and is built only where the ids of the answered operations
/// are in hand (TG-SYNC-004). A handler-level counter is not such an id, and a
/// client that received one would retire pending writes the server never applied.
async fn send_operation_response(resp: OperationResponse, tx: &mpsc::Sender<OutboundMessage>) {
    match resp {
        OperationResponse::Message(msg) => {
            if let Ok(bytes) = rmp_serde::to_vec_named(&*msg) {
                let _ = tx.send(OutboundMessage::Binary(bytes)).await;
            }
        }
        OperationResponse::Messages(msgs) => {
            for msg in msgs {
                if let Ok(bytes) = rmp_serde::to_vec_named(&msg) {
                    let _ = tx.send(OutboundMessage::Binary(bytes)).await;
                }
            }
        }
        OperationResponse::Empty => {
            // No response needed
        }
        // The fixture-only acknowledgement carries a dispatcher counter, not an
        // operation id, so it must never become a frame. The variant exists in
        // test builds only, hence the gate on this arm.
        #[cfg(test)]
        OperationResponse::Ack { .. } => {}
        OperationResponse::NotImplemented {
            service_name,
            call_id: _,
        } => {
            let err_msg = TopGunMessage::Error {
                payload: ErrorPayload {
                    code: 501,
                    message: format!("not implemented: {service_name}"),
                    details: None,
                },
            };
            if let Ok(bytes) = rmp_serde::to_vec_named(&err_msg) {
                let _ = tx.send(OutboundMessage::Binary(bytes)).await;
            }
        }
    }
}

/// Awaits the outbound task's clean exit, then aborts it if it is wedged.
///
/// The outbound task can stall on a writer whose TCP send-buffer is full (a
/// slow or dead client). We give it a bounded window to drain and close
/// gracefully; if it does not, `abort()` reclaims the task so it cannot linger
/// holding the socket sender and receiver until the OS TCP layer errors.
async fn join_outbound_with_timeout(handle: tokio::task::JoinHandle<()>) {
    let mut handle = handle;
    if tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle)
        .await
        .is_err()
    {
        handle.abort();
    }
}

/// Drains the outbound mpsc channel and writes each message to the socket.
///
/// Every frame of a queued message, from [`outbound_frames`], is written
/// before the next message is taken off the queue. So wire order is queue
/// order and no two messages are ever merged. On whole-message framing each
/// queued message is one WebSocket message of any size (`TG-WS-001`); on a
/// chunk-capable connection a message longer than the chunk length is a run
/// of `CHUNK` messages instead (`TG-WS-002`). Draining ready messages with
/// `try_recv` only saves a wake-up per message.
async fn outbound_task<S>(
    mut sender: S,
    mut rx: mpsc::Receiver<OutboundMessage>,
    framing: OutboundFraming,
) where
    S: Sink<Message> + Unpin,
{
    while let Some(msg) = rx.recv().await {
        if send_outbound_message(&mut sender, msg, framing)
            .await
            .is_err()
        {
            break;
        }
        while let Ok(msg) = rx.try_recv() {
            if send_outbound_message(&mut sender, msg, framing)
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // Gracefully close the WebSocket write half.
    let _ = sender.close().await;
}

/// Sends a single outbound message to the WebSocket sender.
///
/// Returns `Ok(())` on success or `Err(())` when the connection should
/// be torn down (send error or Close message).
async fn send_outbound_message<S>(
    sender: &mut S,
    msg: OutboundMessage,
    framing: OutboundFraming,
) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    match msg {
        OutboundMessage::Binary(data) => {
            for (frame, flush) in outbound_frames(Bytes::from(data), framing) {
                let written = if flush {
                    sender.send(Message::Frame(frame)).await
                } else {
                    sender.feed(Message::Frame(frame)).await
                };
                if written.is_err() {
                    return Err(());
                }
            }
        }
        OutboundMessage::Close(reason) => {
            let close_frame = reason.map(|r| CloseFrame {
                code: CloseCode::Normal,
                reason: r.into(),
            });
            let _ = sender.send(Message::Close(close_frame)).await;
            return Err(());
        }
    }
    Ok(())
}

/// The most bytes a `CHUNK`'s header can take: a three-entry map, its keys,
/// `"CHUNK"`, the flag and a `bin32` length header are 28 bytes.
const CHUNK_HEADER_MAX: usize = 32;

/// How a client connection's writer frames what it dequeues, fixed when the
/// writer is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutboundFraming {
    /// Largest payload of one frame (`TG-WS-001`).
    fragment_len: usize,
    /// The effective chunk length on a chunk-capable connection, `None` for
    /// whole messages (`TG-WS-002`).
    chunk_len: Option<usize>,
}

impl OutboundFraming {
    /// The framing of a connection, from its settings and whether it opted in
    /// to chunked delivery.
    ///
    /// The chunk length is clamped so that it is never 0 (which would never
    /// consume a message) and so that a `CHUNK`'s header and data each fit one
    /// frame budget: every `CHUNK` is then exactly two frames. When the budget
    /// cannot hold a header (`outbound_fragment_len() < 33`), no connection is
    /// chunked. The `bin32` cap only matters for a write buffer above 4 GiB.
    fn new(connection: &ConnectionConfig, chunked: bool) -> Self {
        let fragment_len = connection.outbound_fragment_len();
        let chunk_len = (chunked && fragment_len > CHUNK_HEADER_MAX).then(|| {
            connection
                .outbound_chunk_len
                .clamp(1, fragment_len - CHUNK_HEADER_MAX)
                .min(u32::MAX as usize)
        });
        Self {
            fragment_len,
            chunk_len,
        }
    }
}

/// The frames of one outbound binary message under `framing`, each with
/// whether it ends a flush.
///
/// A message of at most the chunk length, and every message on whole-message
/// framing, gives the frames of [`message_frames`], each flushed: the bytes
/// of `TG-WS-001`. A longer message on a chunk-capable connection gives its
/// `CHUNK`s from [`chunk_frames`], one flush per `CHUNK`.
fn outbound_frames(payload: Bytes, framing: OutboundFraming) -> OutboundFrames {
    match framing.chunk_len {
        Some(chunk_len) if payload.len() > chunk_len => {
            OutboundFrames::Chunked(chunk_frames(payload, chunk_len))
        }
        _ => OutboundFrames::Whole(message_frames(payload, framing.fragment_len)),
    }
}

/// Iterator behind [`outbound_frames`].
enum OutboundFrames {
    Whole(MessageFrames),
    Chunked(ChunkFrames),
}

impl Iterator for OutboundFrames {
    type Item = (Frame, bool);

    fn next(&mut self) -> Option<(Frame, bool)> {
        match self {
            Self::Whole(frames) => frames.next().map(|frame| (frame, true)),
            Self::Chunked(frames) => frames.next(),
        }
    }
}

/// The `CHUNK` messages of one outbound message (`TG-WS-002`), as frames.
///
/// `CHUNK` `i` carries bytes `[i·chunk_len, min((i+1)·chunk_len, n))` of the
/// message, so none is empty, also at an exact multiple, and only the last
/// has `last = true`. Each `CHUNK` is the MsgPack map
/// `{"type": "CHUNK", "last": bool, "data": bin}` written as two frames: a
/// `Binary` frame without `fin` holding everything up to the `data` bytes,
/// then a `Continue` frame with `fin` whose payload is a zero-copy view of the
/// message buffer. The header frame is not flushed on its own, so each `CHUNK`
/// costs one flush.
fn chunk_frames(payload: Bytes, chunk_len: usize) -> ChunkFrames {
    ChunkFrames {
        rest: payload,
        chunk_len,
        data: None,
    }
}

/// Iterator behind [`chunk_frames`]; per `CHUNK` it allocates only the
/// header.
struct ChunkFrames {
    rest: Bytes,
    chunk_len: usize,
    /// The data frame of the `CHUNK` whose header was just yielded.
    data: Option<Frame>,
}

impl Iterator for ChunkFrames {
    type Item = (Frame, bool);

    fn next(&mut self) -> Option<(Frame, bool)> {
        if let Some(data) = self.data.take() {
            return Some((data, true));
        }
        if self.rest.is_empty() {
            return None;
        }
        let data = self.rest.split_to(self.chunk_len.min(self.rest.len()));
        let header = chunk_header(self.rest.is_empty(), data.len());
        self.data = Some(Frame::message(data, OpCode::Data(OpData::Continue), true));
        Some((
            Frame::message(header, OpCode::Data(OpData::Binary), false),
            false,
        ))
    }
}

/// The MsgPack bytes of a `CHUNK` up to its `data` bytes, with the smallest
/// `bin` length header that fits `len`.
fn chunk_header(last: bool, len: usize) -> Bytes {
    let mut header = Vec::with_capacity(CHUNK_HEADER_MAX);
    header.push(0x83); // fixmap, three entries
    header.extend_from_slice(b"\xa4type\xa5CHUNK\xa4last");
    header.push(if last { 0xc3 } else { 0xc2 });
    header.extend_from_slice(b"\xa4data");
    if let Ok(len) = u8::try_from(len) {
        header.push(0xc4);
        header.push(len);
    } else if let Ok(len) = u16::try_from(len) {
        header.push(0xc5);
        header.extend_from_slice(&len.to_be_bytes());
    } else {
        let len = u32::try_from(len).expect("the chunk length is capped at u32::MAX");
        header.push(0xc6);
        header.extend_from_slice(&len.to_be_bytes());
    }
    Bytes::from(header)
}

/// The frames of one outbound binary message, each carrying at most
/// `fragment_len` bytes (`TG-WS-001`).
///
/// A message of at most `fragment_len` bytes, an empty one included, is one
/// `Binary` frame with `fin` set: the frame tungstenite builds for
/// `Message::Binary`, so its bytes on the wire are unchanged, and its payload
/// is moved rather than sliced. A longer message is a `Binary` frame without
/// `fin`, then `Continue` frames, the last with `fin` (RFC 6455 section 5.4).
/// Each payload is a zero-copy view of the one message buffer, and none is
/// empty, also when the length is an exact multiple of `fragment_len`.
fn message_frames(payload: Bytes, fragment_len: usize) -> MessageFrames {
    MessageFrames {
        rest: Some(payload),
        // A zero length would never consume the payload.
        fragment_len: fragment_len.max(1),
        first: true,
    }
}

/// Iterator behind [`message_frames`]; it allocates nothing per message.
struct MessageFrames {
    rest: Option<Bytes>,
    fragment_len: usize,
    first: bool,
}

impl Iterator for MessageFrames {
    type Item = Frame;

    fn next(&mut self) -> Option<Frame> {
        let mut rest = self.rest.take()?;
        let opcode = OpCode::Data(if self.first {
            OpData::Binary
        } else {
            OpData::Continue
        });
        self.first = false;
        if rest.len() <= self.fragment_len {
            return Some(Frame::message(rest, opcode, true));
        }
        let head = rest.split_to(self.fragment_len);
        self.rest = Some(rest);
        Some(Frame::message(head, opcode, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::delta_buffer::DeltaBuffer;
    use crate::query::window::LiveWindow;
    use crate::service::domain::journal::{JournalStore, JournalSubscription};
    use crate::service::domain::query::{QueryRegistry, QuerySubscription};
    use crate::service::domain::search::{
        HybridSearchSubscription, SearchSubscription, SubscriptionRegistry,
    };
    use std::collections::BTreeSet;
    use topgun_core::messages::base::Query;
    use topgun_core::messages::search::SearchOptions;
    use topgun_core::messages::{BatchMessage, ClientOp, OpBatchMessage, OpBatchPayload};

    /// Frames `body` as one length-prefixed batch inner item (4-byte big-endian
    /// length header + body), matching the wire framing the unpacker expects.
    fn frame_inner_item(body: &[u8]) -> Vec<u8> {
        let len = u32::try_from(body.len()).expect("inner item fits in u32");
        let mut out = len.to_be_bytes().to_vec();
        out.extend_from_slice(body);
        out
    }

    /// Concatenates `n` minimal (empty-body) framed inner items.
    fn pack_n_minimal_items(n: usize) -> Vec<u8> {
        let mut data = Vec::new();
        for _ in 0..n {
            data.extend_from_slice(&frame_inner_item(&[]));
        }
        data
    }

    /// The cost of a frame is the number of ops it carries, however they are
    /// packed. For a transport `Batch` the declared `count` is
    /// attacker-controlled and is not the cost basis — it never reaches the
    /// unpacker, which is handed the envelope's bytes alone.
    #[test]
    fn inbound_op_cost_counts_batch_ops() {
        // A non-batch message costs one token.
        let ping = TopGunMessage::Ping(topgun_core::messages::PingData { timestamp: 0 });
        assert_eq!(message_cost(&ping), 1);

        // An OpBatch costs its op count.
        let three_ops = TopGunMessage::OpBatch(OpBatchMessage {
            payload: OpBatchPayload {
                ops: vec![
                    ClientOp::default(),
                    ClientOp::default(),
                    ClientOp::default(),
                ],
                write_concern: None,
                timeout: None,
            },
        });
        assert_eq!(message_cost(&three_ops), 3);

        // An empty OpBatch still costs one token (cannot be used to spin for free).
        let empty = TopGunMessage::OpBatch(OpBatchMessage {
            payload: OpBatchPayload::default(),
        });
        assert_eq!(message_cost(&empty), 1);

        // A transport Batch declaring count=1 but packing 50 framed inner items
        // is charged 50, NOT 1 — the declared count is not trusted.
        let amplified = BatchMessage {
            count: 1,
            data: pack_n_minimal_items(50),
        };
        assert_eq!(UnpackedEnvelope::unpack(&amplified.data).cost(), 50);

        // An empty transport Batch (no framed items) still costs one token.
        let empty_batch = BatchMessage {
            count: 0,
            data: vec![],
        };
        assert_eq!(UnpackedEnvelope::unpack(&empty_batch.data).cost(), 1);

        // A transport Batch declaring count=99 but packing only 3 framed items
        // is charged 3 — the cost follows the actual packed count, not the lie.
        let over_declared = BatchMessage {
            count: 99,
            data: pack_n_minimal_items(3),
        };
        assert_eq!(UnpackedEnvelope::unpack(&over_declared.data).cost(), 3);
    }

    /// Disconnect cleanup must drain a connection's standing subscriptions from
    /// every registry wired into `AppState`, not just lock/topic/counter. This
    /// drives `release_session_state` directly and asserts each of the four
    /// later-added registries reports zero subscriptions for the disconnected
    /// connection — the regression that left query/journal/search/hybrid
    /// subscriptions leaked on disconnect.
    #[test]
    fn release_session_state_clears_query_journal_search_hybrid_subscriptions() {
        let conn = ConnectionId(7);

        let query_registry = Arc::new(QueryRegistry::new());
        let journal_store = Arc::new(JournalStore::new(100));
        let search_registry = Arc::new(SubscriptionRegistry::<SearchSubscription>::new());
        let hybrid_registry = Arc::new(SubscriptionRegistry::<HybridSearchSubscription>::new());

        // One subscription per registry, all owned by the same connection.
        let query = Query::default();
        let live_window = Arc::new(LiveWindow::new(
            query.sort.clone().unwrap_or_default(),
            query.limit,
        ));
        query_registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn,
            map_name: "users".to_string(),
            query,
            live_window,
            fields: None,
            delta_buffer: Arc::new(DeltaBuffer::new(64)),
        });

        journal_store.subscribe(
            "j-1".to_string(),
            JournalSubscription {
                connection_id: conn,
                map_name: Some("users".to_string()),
                types: None,
                principal: None,
            },
        );

        search_registry.register(SearchSubscription::new(
            "s-1".to_string(),
            conn,
            "users".to_string(),
            "hello".to_string(),
            SearchOptions::default(),
        ));

        hybrid_registry.register(HybridSearchSubscription::new(
            "h-1".to_string(),
            conn,
            "users".to_string(),
            "hello".to_string(),
            Vec::new(),
            10,
            None,
            None,
            false,
            None,
        ));

        // Sanity: each registry holds the connection's subscription before disconnect.
        assert_eq!(query_registry.subscription_count(), 1);
        assert_eq!(journal_store.subscription_count_for_connection(conn), 1);
        assert_eq!(search_registry.get_subscriptions_for_map("users").len(), 1);
        assert_eq!(hybrid_registry.get_subscriptions_for_map("users").len(), 1);

        let state = AppState {
            query_registry: Some(Arc::clone(&query_registry)),
            journal_store: Some(Arc::clone(&journal_store)),
            search_registry: Some(Arc::clone(&search_registry)),
            hybrid_search_registry: Some(Arc::clone(&hybrid_registry)),
            ..AppState::for_test()
        };

        release_session_state(&state, conn);

        assert_eq!(
            query_registry.subscription_count(),
            0,
            "query registry retained subscription after disconnect"
        );
        assert_eq!(
            journal_store.subscription_count_for_connection(conn),
            0,
            "journal store retained subscription after disconnect"
        );
        assert_eq!(
            search_registry.get_subscriptions_for_map("users").len(),
            0,
            "search registry retained subscription after disconnect"
        );
        assert_eq!(
            hybrid_registry.get_subscriptions_for_map("users").len(),
            0,
            "hybrid-search registry retained subscription after disconnect"
        );
    }

    // -----------------------------------------------------------------------
    // Verdict fold: per-operation attribution of a permanent sub-batch failure
    // -----------------------------------------------------------------------

    use crate::network::config::ConnectionConfig;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::config::ServerConfig;
    use crate::service::dispatch::DispatchConfig;
    use crate::service::domain::crdt::CrdtService;
    use crate::service::domain::schema::SchemaService;
    use crate::service::middleware::pipeline::build_operation_pipeline;
    use crate::service::operation::service_names;
    use crate::service::policy::expr_parser::parse_permission_expr;
    use crate::service::policy::{
        InMemoryPolicyStore, PermissionAction, PermissionPolicy, PolicyEffect, PolicyEvaluator,
        PolicyStore,
    };
    use crate::service::router::OperationRouter;
    use crate::service::security::{SecurityConfig, WriteAdmission};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::record::RecordValue;
    use crate::traits::SchemaProvider;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use topgun_core::messages::base::{PredicateNode, PredicateOp};
    use topgun_core::{
        FieldDef, FieldType, LWWRecord, MapSchema, Principal, SystemClock, Timestamp, HLC,
    };

    /// The map every fold test writes to. One map for every op of a sub-batch is
    /// deliberate: it is what makes the fixture model the real refusal. Policy is
    /// evaluated per `(map_name, op_data)`, so two ops on the same map can get
    /// different decisions and the map name does NOT identify the denied write —
    /// a fixture that put the bad op on its own map would let a wrong
    /// implementation (split the sub-batch by map name) pass.
    const FOLD_MAP: &str = "notes";

    /// A map with a registered schema, used by the `SchemaInvalid` case.
    const TYPED_MAP: &str = "typed-notes";

    /// A whole pipeline — router, middleware, dispatcher, CRDT service — wired the
    /// way the server wires it, so the fold is exercised against the real
    /// Authorization middleware rather than a stub that returns `Forbidden`.
    struct FoldFixture {
        classify_svc: OperationService,
        dispatcher: Arc<PartitionDispatcher>,
        factory: Arc<RecordStoreFactory>,
        journal: Arc<JournalStore>,
        conn_id: ConnectionId,
        /// Held so the connection stays registered: the CRDT service snapshots
        /// its metadata on every batch that carries a `connection_id`.
        _connection: Arc<ConnectionHandle>,
        _outbound: mpsc::Receiver<OutboundMessage>,
        /// How many operations reached the domain service. One per sub-batch on
        /// the optimistic pass, plus one per singleton the fallback re-dispatched
        /// — which is what makes "the fallback was never entered" checkable.
        service_calls: Arc<std::sync::atomic::AtomicUsize>,
        /// What the domain service was handed, in arrival order.
        received: ReceivedOps,
    }

    impl FoldFixture {
        /// The distinct partition labels the domain service was handed, sorted.
        ///
        /// The label is what the dispatcher picks a worker by, so this is the
        /// routing decision as the service saw it. That a label reaches its
        /// worker is the dispatcher's own test, not this fixture's.
        fn received_partitions(&self) -> Vec<Option<u32>> {
            let mut partitions: Vec<Option<u32>> = self
                .received
                .lock()
                .iter()
                .map(|(partition_id, _)| *partition_id)
                .collect();
            partitions.sort_unstable();
            partitions.dedup();
            partitions
        }

        /// The dispatch context the transports will build, with a write concern
        /// and a timeout set so the singleton pass is exercised carrying them.
        fn cx(&self) -> OpBatchDispatchContext<'_> {
            OpBatchDispatchContext {
                classify_svc: &self.classify_svc,
                dispatcher: &self.dispatcher,
                transport: TransportKind::WebSocket,
                caller_origin: CallerOrigin::Client,
                client_id: None,
                principal: Some(fold_principal()),
                connection_id: Some(self.conn_id),
                write_concern: Some(WriteConcern::APPLIED),
                timeout: Some(5_000),
            }
        }

        /// Whether the durable store holds a live value for `key`.
        async fn holds(&self, map_name: &str, key: &str) -> bool {
            self.factory
                .get_or_create(map_name, hash_to_partition(key))
                .get(key, false)
                .await
                .expect("store read")
                .is_some()
        }

        /// The stamp the durable store holds for an LWW `key`, if it holds one.
        async fn stored_stamp(&self, map_name: &str, key: &str) -> Option<Timestamp> {
            let record = self
                .factory
                .get_or_create(map_name, hash_to_partition(key))
                .get(key, false)
                .await
                .expect("store read")?;
            match record.value {
                RecordValue::Lww { timestamp, .. } => Some(timestamp),
                _ => None,
            }
        }

        /// Keys of every journalled event, in apply order.
        fn journalled_keys(&self) -> Vec<String> {
            self.journal
                .read(0, 1000, None, None)
                .0
                .into_iter()
                .map(|event| event.key)
                .collect()
        }
    }

    /// The authenticated writer every fold test dispatches as.
    fn fold_principal() -> Principal {
        Principal {
            id: "writer-1".to_string(),
            roles: vec!["user".to_string()],
            tenant: None,
        }
    }

    /// The failure a [`SheddingCrdt`] stages for the batch it is aimed at.
    ///
    /// `OperationError` is not `Clone`, and the fixture's service is, so the
    /// staged failure is carried as this discriminant and constructed per call.
    #[derive(Clone, Copy)]
    enum StagedFailure {
        Overloaded,
        Unauthorized,
    }

    impl StagedFailure {
        fn error(self) -> OperationError {
            match self {
                Self::Overloaded => OperationError::Overloaded,
                Self::Unauthorized => OperationError::Unauthorized,
            }
        }
    }

    /// One entry per operation the domain service was handed: the partition it
    /// was labelled with and how many client operations it carried.
    type ReceivedOps = Arc<parking_lot::Mutex<Vec<(Option<u32>, usize)>>>;

    /// A CRDT service that answers its staged failure for any batch or single
    /// write touching `shed_key`, and delegates everything else to the real one.
    /// It also records what it was handed, before deciding either way.
    ///
    /// The production `LoadShedLayer` sheds by semaphore occupancy, which cannot
    /// be aimed at one operation of a sequentially re-dispatched fallback, and
    /// `Unauthorized` is raised by middleware the fixture does not drive per
    /// operation either. What the transport branches on is the error's
    /// disposition, not where it was raised, so the failure is staged at the
    /// service instead — the same `OperationError` value the real raiser
    /// produces.
    #[derive(Clone)]
    struct SheddingCrdt {
        inner: Arc<CrdtService>,
        shed_key: Option<String>,
        staged: StagedFailure,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        received: ReceivedOps,
    }

    impl tower::Service<Operation> for SheddingCrdt {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<OperationResponse, OperationError>> + Send>,
        >;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let op_count = match &op {
                Operation::OpBatch { payload, .. } => payload.payload.ops.len(),
                _ => 1,
            };
            self.received.lock().push((op.ctx().partition_id, op_count));
            let sheds = self.shed_key.as_ref().is_some_and(|shed_key| match &op {
                Operation::OpBatch { payload, .. } => payload
                    .payload
                    .ops
                    .iter()
                    .any(|client_op| &client_op.key == shed_key),
                // A single write is shed too, so the failure can be aimed at a
                // message that takes the generic classify-and-dispatch route
                // rather than the batch one.
                Operation::ClientOp { payload, .. } => &payload.payload.key == shed_key,
                _ => false,
            });
            if sheds {
                let staged = self.staged;
                return Box::pin(async move { Err(staged.error()) });
            }
            let mut inner = Arc::clone(&self.inner);
            Box::pin(async move { tower::Service::call(&mut inner, op).await })
        }
    }

    /// Builds the fixture with the given policies and optional map schema.
    ///
    /// An empty policy list leaves the store unconfigured, which is the
    /// evaluator's documented allow-all gate — that is how the schema case gets a
    /// permanent failure raised inside the CRDT service rather than by RBAC.
    async fn build_fold_fixture(
        policies: Vec<PermissionPolicy>,
        schema: Option<(&str, MapSchema)>,
    ) -> FoldFixture {
        build_fold_fixture_shedding(policies, schema, None).await
    }

    /// The same fixture, with the domain service made to shed every batch that
    /// touches `shed_key` so a transient failure can be aimed at one operation.
    async fn build_fold_fixture_shedding(
        policies: Vec<PermissionPolicy>,
        schema: Option<(&str, MapSchema)>,
        shed_key: Option<String>,
    ) -> FoldFixture {
        build_fold_fixture_staging(policies, schema, shed_key, StagedFailure::Overloaded).await
    }

    /// The shedding fixture with the staged failure named, for the cases whose
    /// subject is a disposition other than the transient one.
    async fn build_fold_fixture_staging(
        policies: Vec<PermissionPolicy>,
        schema: Option<(&str, MapSchema)>,
        shed_key: Option<String>,
        staged: StagedFailure,
    ) -> FoldFixture {
        let server_config = Arc::new(ServerConfig::default());
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let connection_registry = Arc::new(ConnectionRegistry::new());
        let journal = Arc::new(JournalStore::new(1_000));
        let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            "fold-test-node".to_string(),
            Box::new(SystemClock),
        )));
        let schema_svc = Arc::new(SchemaService::new());
        if let Some((map_name, map_schema)) = schema {
            schema_svc
                .register_schema(map_name, map_schema)
                .await
                .expect("schema registers");
        }
        let crdt = Arc::new(
            CrdtService::new(
                Arc::clone(&factory),
                Arc::clone(&connection_registry),
                Arc::new(WriteAdmission::new(
                    Arc::new(SecurityConfig::default()),
                    Arc::clone(&hlc),
                )),
                Arc::new(QueryRegistry::new()),
                schema_svc,
            )
            .with_journal(Arc::clone(&journal)),
        );

        let policy_store = Arc::new(InMemoryPolicyStore::new());
        for policy in policies {
            policy_store
                .upsert_policy(policy)
                .await
                .expect("policy upsert");
        }
        let evaluator = Arc::new(PolicyEvaluator::new(policy_store));

        // One worker: every sub-batch and every singleton is served by the same
        // pipeline instance, so nothing in these tests depends on which worker a
        // partition happened to land on.
        let dispatch_config = DispatchConfig {
            worker_count: 1,
            channel_buffer_size: 64,
        };
        let service_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let received: ReceivedOps = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let dispatcher = Arc::new(PartitionDispatcher::new(&dispatch_config, || {
            let mut router = OperationRouter::new();
            router.register(
                service_names::CRDT,
                SheddingCrdt {
                    inner: Arc::clone(&crdt),
                    shed_key: shed_key.clone(),
                    staged,
                    calls: Arc::clone(&service_calls),
                    received: Arc::clone(&received),
                },
            );
            build_operation_pipeline(router, &server_config, Some(Arc::clone(&evaluator)))
        }));

        let (connection, outbound) =
            connection_registry.register(ConnectionKind::Client, &ConnectionConfig::default());
        let conn_id = connection.id;

        FoldFixture {
            classify_svc: OperationService::new(hlc, server_config),
            dispatcher,
            factory,
            journal,
            conn_id,
            _connection: connection,
            _outbound: outbound,
            service_calls,
            received,
        }
    }

    /// Allow every write to `FOLD_MAP`, except one whose record carries
    /// `blocked: true`.
    ///
    /// Deny-wins, and the deny is selected by the record's own content, so the
    /// denied op is identified by its data and not by its map.
    fn deny_blocked_records() -> Vec<PermissionPolicy> {
        vec![
            PermissionPolicy {
                id: "allow-notes".to_string(),
                map_pattern: FOLD_MAP.to_string(),
                action: PermissionAction::Write,
                effect: PolicyEffect::Allow,
                condition: None,
            },
            PermissionPolicy {
                id: "deny-blocked".to_string(),
                map_pattern: FOLD_MAP.to_string(),
                action: PermissionAction::Write,
                effect: PolicyEffect::Deny,
                condition: Some(PredicateNode {
                    op: PredicateOp::Eq,
                    attribute: Some("blocked".to_string()),
                    value: Some(rmpv::Value::Boolean(true)),
                    children: None,
                    value_ref: None,
                }),
            },
        ]
    }

    /// A schema requiring a `name` string, so a record without one is rejected by
    /// the CRDT service itself rather than by the middleware.
    fn required_name_schema() -> MapSchema {
        MapSchema {
            version: 1,
            fields: vec![FieldDef {
                name: "name".to_string(),
                required: true,
                field_type: FieldType::String,
                constraints: None,
            }],
            strict: false,
        }
    }

    /// A client PUT carrying `fields` as its record value.
    fn put_op(
        op_id: &str,
        map_name: &str,
        key: &str,
        fields: Vec<(&str, rmpv::Value)>,
    ) -> ClientOp {
        ClientOp {
            id: Some(op_id.to_string()),
            map_name: map_name.to_string(),
            key: key.to_string(),
            op_type: None,
            record: Some(Some(LWWRecord {
                value: Some(rmpv::Value::Map(
                    fields
                        .into_iter()
                        .map(|(name, value)| (rmpv::Value::String(name.into()), value))
                        .collect(),
                )),
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 1,
                    node_id: "fold-test-node".to_string(),
                },
                ttl_ms: None,
            })),
            or_record: None,
            or_tag: None,
            write_concern: None,
            timeout: None,
        }
    }

    /// A record the deny policy matches.
    fn blocked_fields() -> Vec<(&'static str, rmpv::Value)> {
        vec![("blocked", rmpv::Value::Boolean(true))]
    }

    /// A record the deny policy does not match.
    fn allowed_fields(name: &str) -> Vec<(&'static str, rmpv::Value)> {
        vec![("name", rmpv::Value::String(name.into()))]
    }

    /// Op ids named by `OpOutcome::accepted`, in the order the fold produced them.
    fn accepted_ids(outcome: &OpOutcome) -> Vec<String> {
        outcome
            .accepted
            .iter()
            .map(|result| result.op_id.clone())
            .collect()
    }

    /// Destructures the single expected refusal.
    fn sole_refusal(outcome: &OpOutcome) -> (String, u32, &'static str) {
        assert_eq!(
            outcome.refused.len(),
            1,
            "expected exactly one refusal, got {:?}",
            outcome.refused
        );
        let OpVerdict::Refused {
            op_id, code, kind, ..
        } = &outcome.refused[0];
        (op_id.clone(), *code, *kind)
    }

    /// A permanently refused sub-batch applies nothing before the fallback, and
    /// the fallback names the refused op without re-applying the accepted ones
    /// (TG-SYNC-003).
    ///
    /// The journal count is the mechanical guard, not a code-reading: every
    /// applied mutation appends exactly one event, so a singleton re-applying
    /// what the batch pass had already applied would show up as two.
    #[tokio::test]
    async fn permanent_subbatch_failure_applied_nothing_before_fallback() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let ops = vec![
            put_op("101", FOLD_MAP, "k-first", allowed_fields("first")),
            put_op("102", FOLD_MAP, "k-denied", blocked_fields()),
            put_op("103", FOLD_MAP, "k-third", allowed_fields("third")),
        ];
        let partition_id = 7;

        // 1. Zero-applied canary. This is the fold's own batch pass, issued here
        //    so the state BETWEEN the two passes is observable at all — the fold
        //    performs it internally and never exposes the intermediate store.
        let batch_pass = fx.cx().sub_batch_operation(ops.clone(), partition_id);
        let refusal = fx
            .dispatcher
            .dispatch(batch_pass)
            .await
            .expect_err("the whole sub-batch is refused");
        assert!(
            matches!(refusal, OperationError::Forbidden { .. }),
            "expected a middleware refusal, got {refusal:?}"
        );
        for key in ["k-first", "k-denied", "k-third"] {
            assert!(
                !fx.holds(FOLD_MAP, key).await,
                "{key} was applied by a sub-batch that failed permanently"
            );
        }
        assert!(
            fx.journalled_keys().is_empty(),
            "a permanently refused sub-batch journalled something"
        );

        // 2. The fold: two accepted, one refused, nothing else.
        let outcome = attribute_permanent_failure_per_op(vec![(partition_id, ops)], &fx.cx()).await;
        assert_eq!(
            sole_refusal(&outcome),
            ("102".to_string(), 403, "forbidden")
        );
        assert_eq!(accepted_ids(&outcome), vec!["101", "103"]);
        assert!(
            outcome.transient.is_none(),
            "an attributed refusal must not also produce a transient error"
        );
        assert!(
            outcome.batch_error.is_none(),
            "an attributed refusal must not also produce a batch error"
        );

        // 3. Both good ops read back; the refused one did not land.
        assert!(fx.holds(FOLD_MAP, "k-first").await);
        assert!(fx.holds(FOLD_MAP, "k-third").await);
        assert!(!fx.holds(FOLD_MAP, "k-denied").await);

        // 4. Exactly one journal event per accepted op — the double-apply guard.
        assert_eq!(fx.journalled_keys(), vec!["k-first", "k-third"]);
    }

    /// The same fallback attributes a permanent failure raised INSIDE the CRDT
    /// service, not only one raised by the middleware, and carries its own code.
    #[tokio::test]
    async fn permanent_subbatch_failure_applied_nothing_before_fallback_schema_case() {
        let fx = build_fold_fixture(Vec::new(), Some((TYPED_MAP, required_name_schema()))).await;
        let ops = vec![
            put_op("201", TYPED_MAP, "s-first", allowed_fields("first")),
            put_op(
                "202",
                TYPED_MAP,
                "s-invalid",
                vec![("unrelated", rmpv::Value::Integer(1.into()))],
            ),
            put_op("203", TYPED_MAP, "s-third", allowed_fields("third")),
        ];
        let partition_id = 3;

        let batch_pass = fx.cx().sub_batch_operation(ops.clone(), partition_id);
        let refusal = fx
            .dispatcher
            .dispatch(batch_pass)
            .await
            .expect_err("the whole sub-batch is refused");
        assert!(
            matches!(refusal, OperationError::SchemaInvalid { .. }),
            "expected an in-service schema refusal, got {refusal:?}"
        );
        assert!(
            fx.journalled_keys().is_empty(),
            "the validate-all loop must run before the apply loop"
        );

        let outcome = attribute_permanent_failure_per_op(vec![(partition_id, ops)], &fx.cx()).await;
        assert_eq!(
            sole_refusal(&outcome),
            ("202".to_string(), 422, "schema_invalid")
        );
        assert_eq!(accepted_ids(&outcome), vec!["201", "203"]);
        assert_eq!(fx.journalled_keys(), vec!["s-first", "s-third"]);
    }

    /// The singleton pass re-dispatches in the sub-batch's original order.
    ///
    /// Order is observable: each applied op appends one journal event, so the
    /// journal sequence is the dispatch sequence. A fold that re-dispatched in
    /// completion order, or in reverse, would show the two accepted ops swapped.
    #[tokio::test]
    async fn fallback_redispatches_in_original_order() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        // Deliberately ordered so a lexicographic or reversed walk is visible:
        // `z-alpha` is dispatched FIRST and `a-omega` last.
        let ops = vec![
            put_op("301", FOLD_MAP, "z-alpha", allowed_fields("alpha")),
            put_op("302", FOLD_MAP, "m-denied", blocked_fields()),
            put_op("303", FOLD_MAP, "a-omega", allowed_fields("omega")),
        ];

        let outcome = attribute_permanent_failure_per_op(vec![(11, ops)], &fx.cx()).await;

        assert_eq!(sole_refusal(&outcome).0, "302");
        assert_eq!(
            fx.journalled_keys(),
            vec!["z-alpha", "a-omega"],
            "singletons must be re-dispatched in the sub-batch's original order"
        );
        assert_eq!(accepted_ids(&outcome), vec!["301", "303"]);
    }

    /// Reads one labelled counter out of a Prometheus render.
    ///
    /// Matching on the metric name plus every required `label="value"` pair
    /// rather than on a whole formatted line keeps the assertion independent of
    /// the exporter's label ordering.
    fn rendered_labelled_counter(rendered: &str, name: &str, labels: &[(&str, &str)]) -> u64 {
        let matches: Vec<&str> = rendered
            .lines()
            .filter(|line| line.starts_with(&format!("{name}{{")))
            .filter(|line| {
                labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one {name} series for {labels:?}, render was:\n{rendered}"
        );
        matches[0]
            .rsplit(' ')
            .next()
            .expect("a value follows the series")
            .parse()
            .expect("counter renders an integer")
    }

    /// Every `transport × reason` refusal series renders at zero before anything
    /// is refused.
    ///
    /// Without the eager touch the series would simply be absent from a scrape,
    /// and an operator cannot tell an absent series from a healthy zero — which
    /// is how a refusal instrument that never fires passes for a server that
    /// never refuses.
    #[test]
    fn refusal_series_registered_at_zero_for_every_transport_and_reason() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        // The process-wide `Once` fires for whichever recorder is bound first, so
        // the touch itself is driven here rather than through the guard.
        metrics::with_local_recorder(&recorder, touch_client_op_refusal_series);
        let rendered = handle.render();

        for transport in ["ws", "http"] {
            for reason in ALL_ERROR_KINDS {
                assert_eq!(
                    rendered_labelled_counter(
                        &rendered,
                        METRIC_CLIENT_OP_REFUSALS_TOTAL,
                        &[("transport", transport), ("reason", reason)],
                    ),
                    0,
                    "series for {transport}/{reason} must render at zero"
                );
            }
        }
    }

    /// The refusals series counts refusals; the pipeline counter counts pipeline
    /// calls, and its `1 + k` arithmetic is pinned so nobody "fixes" it.
    ///
    /// For one refused op in a three-op sub-batch (k = 1) the refusals series
    /// rises by exactly 1, while `topgun_operation_errors_total` rises by 2: the
    /// failed batch call plus the failed singleton. That is correct for a counter
    /// of pipeline-call errors — `MetricsLayer` sits above `AuthorizationLayer`,
    /// so both calls are observed — and "correcting" it to 1 fails this test.
    #[test]
    fn refused_op_metric_arithmetic() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        // A current-thread runtime driven from inside the binding: the recorder is
        // a THREAD-local, and the dispatcher's worker — where `MetricsLayer` emits
        // — must be polled on the same thread for the emission to be seen.
        let rendered = metrics::with_local_recorder(&recorder, || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime");
            rt.block_on(async {
                let fx = build_fold_fixture(deny_blocked_records(), None).await;

                // Pre-write snapshot: the eager registration is what makes this a
                // readable zero rather than an absent series.
                touch_client_op_refusal_series();
                assert_eq!(
                    rendered_labelled_counter(
                        &handle.render(),
                        METRIC_CLIENT_OP_REFUSALS_TOTAL,
                        &[("transport", "ws"), ("reason", "forbidden")],
                    ),
                    0,
                    "the pre-write snapshot must read zero"
                );

                let ops = vec![
                    put_op("401", FOLD_MAP, "m-first", allowed_fields("first")),
                    put_op("402", FOLD_MAP, "m-denied", blocked_fields()),
                    put_op("403", FOLD_MAP, "m-third", allowed_fields("third")),
                ];
                let outcome = attribute_permanent_failure_per_op(vec![(5, ops)], &fx.cx()).await;
                assert_eq!(
                    sole_refusal(&outcome),
                    ("402".to_string(), 403, "forbidden")
                );
            });
            handle.render()
        });

        assert_eq!(
            rendered_labelled_counter(
                &rendered,
                METRIC_CLIENT_OP_REFUSALS_TOTAL,
                &[("transport", "ws"), ("reason", "forbidden")],
            ),
            1,
            "one refused op must count exactly once on the refusals series"
        );
        assert_eq!(
            rendered_labelled_counter(
                &rendered,
                "topgun_operation_errors_total",
                &[("service", service_names::CRDT), ("error", "forbidden")],
            ),
            2,
            "1 + k pipeline-call errors: the batch call plus the refused singleton"
        );
        assert_eq!(
            rendered_labelled_counter(
                &rendered,
                "topgun_operations_total",
                &[("service", service_names::CRDT), ("outcome", "error")],
            ),
            2,
            "the batch call still returns Err, so the outcome label stays 'error'"
        );
    }

    // -----------------------------------------------------------------------
    // Frames: what one exchange puts on the wire
    // -----------------------------------------------------------------------

    /// `n` distinct keys that all hash to the SAME partition.
    ///
    /// `dispatch_op_batch` groups by `hash_to_partition(key)`, so arbitrary keys
    /// would each form their own single-operation sub-batch and the multi-operation
    /// sub-batch these tests exist to exercise would never be built.
    fn keys_in_one_partition(n: usize) -> Vec<String> {
        let mut buckets: HashMap<u32, Vec<String>> = HashMap::new();
        for i in 0..100_000_u32 {
            let key = format!("fold-key-{i}");
            let bucket = buckets.entry(hash_to_partition(&key)).or_default();
            bucket.push(key);
            if bucket.len() == n {
                return bucket.clone();
            }
        }
        panic!("no partition collected {n} keys");
    }

    /// Runs one `OP_BATCH` through the real transport entry point and returns the
    /// frames it queued, in wire order, as raw bytes.
    ///
    /// Bytes rather than decoded messages because one caller asserts the happy
    /// path is byte-identical to what a server without per-operation verdicts
    /// sent, and a decoded comparison cannot see an added-then-defaulted field.
    async fn dispatch_batch_frames(fx: &FoldFixture, ops: Vec<ClientOp>) -> Vec<Vec<u8>> {
        let (tx, mut rx) = mpsc::channel(64);
        let batch = OpBatchMessage {
            payload: OpBatchPayload {
                ops,
                write_concern: Some(WriteConcern::APPLIED),
                timeout: Some(5_000),
            },
        };
        dispatch_op_batch(
            &batch,
            fx.conn_id,
            Some(fold_principal()),
            &fx.classify_svc,
            &fx.dispatcher,
            &tx,
        )
        .await;
        // The sender the handler holds is a borrow of this one; dropping ours
        // closes the channel so the drain terminates.
        drop(tx);

        let mut frames = Vec::new();
        while let Some(outbound) = rx.recv().await {
            match outbound {
                OutboundMessage::Binary(bytes) => frames.push(bytes),
                OutboundMessage::Close(reason) => {
                    panic!("an op batch must not close the connection: {reason:?}")
                }
            }
        }
        frames
    }

    /// Decodes queued frames back into messages, preserving wire order.
    fn decode_frames(frames: &[Vec<u8>]) -> Vec<TopGunMessage> {
        frames
            .iter()
            .map(|bytes| rmp_serde::from_slice(bytes).expect("a queued frame decodes"))
            .collect()
    }

    /// `(op_id, code, permanent)` of every `OP_REJECTED`, in frame order.
    fn rejections(msgs: &[TopGunMessage]) -> Vec<(String, Option<u32>, bool)> {
        msgs.iter()
            .filter_map(|msg| match msg {
                TopGunMessage::OpRejected(rejected) => Some((
                    rejected.payload.op_id.clone(),
                    rejected.payload.code,
                    rejected.payload.permanent,
                )),
                _ => None,
            })
            .collect()
    }

    /// Every `OP_ACK` payload, in frame order.
    fn ack_payloads(msgs: &[TopGunMessage]) -> Vec<&OpAckPayload> {
        msgs.iter()
            .filter_map(|msg| match msg {
                TopGunMessage::OpAck(ack) => Some(&ack.payload),
                _ => None,
            })
            .collect()
    }

    /// Every `ERROR` payload, in frame order.
    fn error_payloads(msgs: &[TopGunMessage]) -> Vec<&ErrorPayload> {
        msgs.iter()
            .filter_map(|msg| match msg {
                TopGunMessage::Error { payload } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// The operation ids an acknowledgement covers, computed the way the client
    /// computes them: `results` when present, otherwise every batch id at or
    /// below the numeric `lastId`.
    fn ack_coverage(ack: &OpAckPayload, batch_ids: &[&str]) -> BTreeSet<String> {
        if let Some(results) = ack.results.as_ref() {
            return results
                .iter()
                .map(|result| result.op_id.clone())
                .collect::<BTreeSet<String>>();
        }
        let last: u64 = ack
            .last_id
            .parse()
            .expect("a prefix acknowledgement must carry a numeric lastId");
        batch_ids
            .iter()
            .filter(|id| id.parse::<u64>().is_ok_and(|n| n <= last))
            .map(|id| (*id).to_string())
            .collect()
    }

    /// Collects `&str` ids into an owned set, for comparing verdict coverage.
    fn id_set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    /// Every operation of a mixed batch reaches exactly one terminal verdict, and
    /// the two channels partition the batch (TG-SYNC-001 clauses (a) and (b)).
    ///
    /// Coverage is computed the client's way rather than read off the frames'
    /// shape, so an acknowledgement that covered the refused operation by prefix
    /// would fail here even though its `results` did not name it.
    #[tokio::test]
    async fn op_batch_every_op_gets_exactly_one_terminal_verdict() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(3);
        let ops = vec![
            put_op("501", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("502", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("503", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];
        let batch_ids = ["501", "502", "503"];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let rejected = rejections(&msgs);
        assert_eq!(rejected, vec![("502".to_string(), Some(403), true)]);
        let refused: BTreeSet<String> = rejected.into_iter().map(|(op_id, _, _)| op_id).collect();

        let acks = ack_payloads(&msgs);
        assert_eq!(acks.len(), 1, "one exchange acknowledges at most once");
        let covered = ack_coverage(acks[0], &batch_ids);

        assert!(
            covered.is_disjoint(&refused),
            "an operation must not be both acknowledged and refused: {covered:?} vs {refused:?}"
        );
        assert_eq!(
            covered.union(&refused).cloned().collect::<BTreeSet<_>>(),
            id_set(&batch_ids),
            "every id-bearing operation of the batch must carry a verdict"
        );
        assert!(
            error_payloads(&msgs).is_empty(),
            "an attributed refusal is not also a batch error"
        );
        assert!(
            acks[0].last_id.parse::<u64>().is_ok(),
            "a partial ack's lastId must be numeric, got {:?}",
            acks[0].last_id
        );
        assert_eq!(
            acks[0].last_id, "503",
            "lastId is the numeric max accepted id"
        );
    }

    /// A batch in which every operation is refused is answered by the
    /// `OP_REJECTED` frames and nothing else (TG-SYNC-001 clause (b')).
    ///
    /// The last rejection terminates the exchange: there is nothing to
    /// acknowledge, and an `ERROR` frame would report a batch-level failure for
    /// an exchange already explained operation by operation.
    #[tokio::test]
    async fn op_batch_all_refused_emits_rejections_and_no_ack() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(2);
        let ops = vec![
            put_op("601", FOLD_MAP, &keys[0], blocked_fields()),
            put_op("602", FOLD_MAP, &keys[1], blocked_fields()),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        assert!(
            ack_payloads(&msgs).is_empty(),
            "an all-refused batch has nothing to acknowledge"
        );
        assert!(
            error_payloads(&msgs).is_empty(),
            "the refusals already explain the exchange"
        );
        let rejected = rejections(&msgs);
        assert_eq!(rejected.len(), 2, "one rejection per operation");
        let named: BTreeSet<String> = rejected.iter().map(|(op_id, _, _)| op_id.clone()).collect();
        assert_eq!(
            named,
            id_set(&["601", "602"]),
            "each operation is named in exactly one rejection"
        );
        for (op_id, code, permanent) in rejected {
            assert_eq!(code, Some(403), "{op_id} was refused by policy");
            assert!(permanent, "{op_id} must not be presented as retryable");
        }
    }

    /// A transient failure during the fallback leaves the operations after it
    /// with no verdict at all, and acknowledges nothing (TG-SYNC-001 clause (c)).
    ///
    /// Two predicates carry the clause. The rejections already determined are
    /// delivered, and NO acknowledgement is sent — and since `handleOpAck` is the
    /// only client-side site that marks an operation synced, zero `OP_ACK` on the
    /// wire is precisely "no operation of this exchange was marked synced". The
    /// accepted-but-unacknowledged operations stay pending and are re-sent, which
    /// is safe: a permanently failed sub-batch applied nothing (TG-SYNC-003) and
    /// re-applying an accepted LWW write is idempotent.
    #[tokio::test]
    async fn op_batch_transient_mid_fallback_emits_no_ack() {
        let keys = keys_in_one_partition(3);
        let fx =
            build_fold_fixture_shedding(deny_blocked_records(), None, Some(keys[2].clone())).await;
        let ops = vec![
            put_op("701", FOLD_MAP, &keys[0], blocked_fields()),
            put_op("702", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("703", FOLD_MAP, &keys[2], allowed_fields("shed")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let named: BTreeSet<String> = rejections(&msgs)
            .into_iter()
            .map(|(op_id, _, _)| op_id)
            .collect();
        assert_eq!(
            named,
            id_set(&["701", "702"]),
            "the rejections settled before the transient failure are still delivered"
        );
        assert!(
            ack_payloads(&msgs).is_empty(),
            "no operation of this exchange may be acknowledged"
        );
    }

    /// Every rejection reaches the client ahead of the acknowledgement, and the
    /// acknowledgement never covers a refused operation (TG-SYNC-002).
    ///
    /// The ordering rests on FIFO delivery of the connection's outbound channel,
    /// so it is asserted on frame positions in that channel.
    #[tokio::test]
    async fn op_rejected_precedes_ack_and_ack_excludes_refused_op() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(3);
        let ops = vec![
            put_op("801", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("802", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("803", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let rejected_at: Vec<usize> = msgs
            .iter()
            .enumerate()
            .filter(|(_, msg)| matches!(msg, TopGunMessage::OpRejected(_)))
            .map(|(index, _)| index)
            .collect();
        let acked_at: Vec<usize> = msgs
            .iter()
            .enumerate()
            .filter(|(_, msg)| matches!(msg, TopGunMessage::OpAck(_)))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(rejected_at.len(), 1);
        assert_eq!(acked_at.len(), 1);
        assert!(
            rejected_at[0] < acked_at[0],
            "OP_REJECTED must be queued before OP_ACK, got {rejected_at:?} then {acked_at:?}"
        );

        let ack = ack_payloads(&msgs)[0];
        assert!(
            !ack_coverage(ack, &["801", "802", "803"]).contains("802"),
            "the acknowledgement must not cover the refused operation"
        );
    }

    /// A batch nobody refused anything in is acknowledged with the exact bytes a
    /// server without per-operation verdicts sent (AC6, constraint C7).
    ///
    /// Byte equality against a hand-built acknowledgement, not a field-by-field
    /// comparison: an added-but-defaulted field is invisible to the latter and is
    /// exactly what would change the hot path's wire output.
    #[tokio::test]
    async fn uniformly_accepted_batch_ack_is_byte_identical() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(3);
        let ops = vec![
            put_op("901", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("902", FOLD_MAP, &keys[1], allowed_fields("second")),
            put_op("903", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];

        let frames = dispatch_batch_frames(&fx, ops).await;

        assert_eq!(frames.len(), 1, "the happy path emits exactly one frame");
        let expected = TopGunMessage::OpAck(OpAckMessage {
            payload: OpAckPayload {
                last_id: "903".to_string(),
                achieved_level: Some(WriteConcern::APPLIED),
                ..Default::default()
            },
        });
        assert_eq!(
            frames[0],
            rmp_serde::to_vec_named(&expected).expect("expected ack serializes"),
            "the uniformly accepted path must not change a single byte"
        );

        let msgs = decode_frames(&frames);
        assert!(rejections(&msgs).is_empty());
        assert!(ack_payloads(&msgs)[0].results.is_none(), "no results field");
    }

    /// A transient sub-batch failure is reported as the retryable `ERROR` frame,
    /// is never presented as a refusal, and does not enter the fallback at all.
    ///
    /// "No fallback" is asserted mechanically: the service is called exactly once
    /// — the batch pass — where a fallback would have called it once per
    /// operation. `Overloaded` is transient, so retrying is the correct client
    /// response and retiring any of these operations would be wrong.
    #[tokio::test]
    async fn transient_subbatch_yields_429_error_and_no_fallback() {
        let keys = keys_in_one_partition(3);
        let fx = build_fold_fixture_shedding(Vec::new(), None, Some(keys[1].clone())).await;
        let ops = vec![
            put_op("1001", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("1002", FOLD_MAP, &keys[1], allowed_fields("shed")),
            put_op("1003", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let errors = error_payloads(&msgs);
        assert_eq!(
            errors.len(),
            1,
            "one batch-level error, not one per operation"
        );
        assert_eq!(errors[0].code, 429);
        assert!(
            rejections(&msgs).is_empty(),
            "a transient failure is never a refusal"
        );
        assert!(
            ack_payloads(&msgs).is_empty(),
            "nothing may be acknowledged for a sub-batch that failed"
        );
        assert_eq!(
            fx.service_calls.load(Ordering::Relaxed),
            1,
            "a transient sub-batch failure must not be re-dispatched per operation"
        );
    }

    /// `Unauthorized` is classified transient, so a sub-batch that fails with it
    /// is reported as the retryable 401 `ERROR` frame — never as a refusal, and
    /// never acknowledged.
    ///
    /// This is the third assertion of the per-op verdict contract: the two
    /// permanent-refusal paths are proven elsewhere, and this is the case that
    /// says an auth failure is NOT one of them. "No fallback" is asserted the
    /// same way its `Overloaded` twin asserts it — the service is called exactly
    /// once, where a fallback would have called it once per operation — because
    /// a fallback here would re-dispatch operations the server never refused and
    /// hand each one a permanent verdict it has not earned.
    #[tokio::test]
    async fn unauthorized_subbatch_yields_401_error_and_no_rejection() {
        let keys = keys_in_one_partition(3);
        let fx = build_fold_fixture_staging(
            Vec::new(),
            None,
            Some(keys[1].clone()),
            StagedFailure::Unauthorized,
        )
        .await;
        let ops = vec![
            put_op("1201", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("1202", FOLD_MAP, &keys[1], allowed_fields("unauthorized")),
            put_op("1203", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let errors = error_payloads(&msgs);
        assert_eq!(
            errors.len(),
            1,
            "one batch-level error, not one per operation"
        );
        assert_eq!(errors[0].code, 401);
        assert!(
            rejections(&msgs).is_empty(),
            "an auth failure is not a per-operation refusal"
        );
        assert!(
            ack_payloads(&msgs).is_empty(),
            "nothing may be acknowledged for a sub-batch that failed"
        );
        assert_eq!(
            fx.service_calls.load(Ordering::Relaxed),
            1,
            "an Unauthorized sub-batch must not be re-dispatched per operation"
        );
    }

    /// A transient failure raised by the THIRD singleton of a fallback yields the
    /// 429 `ERROR` frame with no acknowledgement, behind the rejections the
    /// earlier singletons already settled.
    ///
    /// This is the server-side half of the clause (c) test above and must not
    /// drift from it: both establish "no operation was marked synced" the same
    /// way, by asserting no acknowledgement is sent at all.
    #[tokio::test]
    async fn transient_mid_fallback_yields_429_after_earlier_rejections() {
        let keys = keys_in_one_partition(3);
        let fx =
            build_fold_fixture_shedding(deny_blocked_records(), None, Some(keys[2].clone())).await;
        let ops = vec![
            put_op("1101", FOLD_MAP, &keys[0], blocked_fields()),
            put_op("1102", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("1103", FOLD_MAP, &keys[2], allowed_fields("shed")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let error_at = msgs
            .iter()
            .position(|msg| matches!(msg, TopGunMessage::Error { .. }))
            .expect("the transient failure is reported as an ERROR frame");
        let TopGunMessage::Error { payload } = &msgs[error_at] else {
            unreachable!("position matched an ERROR frame")
        };
        assert_eq!(payload.code, 429);

        let last_rejection = msgs
            .iter()
            .rposition(|msg| matches!(msg, TopGunMessage::OpRejected(_)))
            .expect("the earlier singletons were refused");
        assert!(
            last_rejection < error_at,
            "rejections already settled must precede the ERROR frame"
        );
        assert_eq!(rejections(&msgs).len(), 2);
        assert!(
            ack_payloads(&msgs).is_empty(),
            "no ack at all, even though nothing before the transient was accepted"
        );
    }

    // -----------------------------------------------------------------------
    // No frame where there is no client operation to give a verdict on
    // -----------------------------------------------------------------------

    /// Closes the channel and decodes every frame queued on it, in wire order.
    async fn queued_messages(
        tx: mpsc::Sender<OutboundMessage>,
        mut rx: mpsc::Receiver<OutboundMessage>,
    ) -> Vec<TopGunMessage> {
        // The drain below ends only once every sender is gone.
        drop(tx);
        let mut frames = Vec::new();
        while let Some(outbound) = rx.recv().await {
            match outbound {
                OutboundMessage::Binary(bytes) => frames.push(bytes),
                OutboundMessage::Close(reason) => {
                    panic!("the exchange must not close the connection: {reason:?}")
                }
            }
        }
        decode_frames(&frames)
    }

    /// An `OP_BATCH` with no operations is answered with no frame.
    ///
    /// An acknowledgement here names no operation, and a client reads a
    /// non-numeric `lastId` as "every pending operation was accepted" — so the
    /// one frame this exchange could send is the one that deletes writes the
    /// server never saw.
    #[tokio::test]
    async fn empty_op_batch_emits_no_frame() {
        let fx = build_fold_fixture(Vec::new(), None).await;

        let msgs = decode_frames(&dispatch_batch_frames(&fx, Vec::new()).await);

        assert!(
            msgs.is_empty(),
            "an empty op batch has no operation to give a verdict on, got {msgs:?}"
        );
    }

    /// An empty `OP_BATCH` carried inside a `BATCH` envelope is answered with
    /// no frame either, and never reaches the domain service.
    ///
    /// An inner message takes the entry a top-level one takes, so the empty
    /// batch stops at the same branch: nothing is dispatched and nothing is
    /// sent. Silence alone would also be what a dropped inner batch looks like,
    /// so the same envelope carries a second, non-empty batch that has to be
    /// dispatched and acknowledged.
    #[tokio::test]
    async fn nested_empty_op_batch_emits_no_frame() {
        let fx = build_fold_fixture(Vec::new(), None).await;
        let control_key = "nested-control-key";
        let control_id = "1601";
        let empty = op_batch_message(Vec::new());
        let control = op_batch_message(vec![put_op(
            control_id,
            FOLD_MAP,
            control_key,
            allowed_fields("control"),
        )]);

        let msgs = decode_frames(&enveloped_frames(&fx, &[empty, control]).await);

        let received = fx.received.lock().clone();
        let calls_for_the_empty_batch = received.iter().filter(|(_, ops)| *ops == 0).count();
        let control_acks = ack_payloads(&msgs)
            .iter()
            .filter(|ack| ack.last_id == control_id)
            .count();
        let control_dispatched = received.contains(&(Some(hash_to_partition(control_key)), 1));
        assert_eq!(
            (
                calls_for_the_empty_batch,
                msgs.len() - control_acks,
                control_dispatched,
                control_acks == 1,
            ),
            (0, 0, true, true),
            "(service calls for the empty batch, frames other than the control's \
             acknowledgement, control batch dispatched, control batch acknowledged); \
             received: {received:?}, frames: {msgs:?}"
        );
    }

    /// The bare acknowledgement response is mapped to no frame.
    ///
    /// It carries only the server's call id — a counter shared by every
    /// connection and message kind. Put on the wire as `lastId`, a client takes
    /// it for one of its own operation ids and deletes every pending write at or
    /// below it.
    #[tokio::test]
    async fn ack_response_variant_emits_no_frame() {
        let (tx, rx) = mpsc::channel(8);

        send_operation_response(OperationResponse::Ack { call_id: 7 }, &tx).await;
        let msgs = queued_messages(tx, rx).await;

        assert!(
            msgs.is_empty(),
            "a call id is not a client operation id and must not reach the wire, got {msgs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // A message inside a `BATCH` envelope is handled like the same message sent
    // on its own
    // -----------------------------------------------------------------------

    /// Wraps `ops` as the `OP_BATCH` message a client sends, with the write
    /// concern and timeout the other transport tests use.
    fn op_batch_message(ops: Vec<ClientOp>) -> TopGunMessage {
        TopGunMessage::OpBatch(OpBatchMessage {
            payload: OpBatchPayload {
                ops,
                write_concern: Some(WriteConcern::APPLIED),
                timeout: Some(5_000),
            },
        })
    }

    /// Wraps one write as the `CLIENT_OP` message a client sends.
    fn client_op_message(op: ClientOp) -> TopGunMessage {
        TopGunMessage::ClientOp(topgun_core::messages::sync::ClientOpMessage { payload: op })
    }

    /// Drains a channel whose handler has returned, as raw frames in wire order.
    ///
    /// Every caller awaits its handler to completion first, so an empty result
    /// is read from a closed channel and never from a wait that ran out.
    async fn queued_frames(
        tx: mpsc::Sender<OutboundMessage>,
        mut rx: mpsc::Receiver<OutboundMessage>,
    ) -> Vec<Vec<u8>> {
        drop(tx);
        let mut frames = Vec::new();
        while let Some(outbound) = rx.recv().await {
            match outbound {
                OutboundMessage::Binary(bytes) => frames.push(bytes),
                OutboundMessage::Close(reason) => {
                    panic!("the exchange must not close the connection: {reason:?}")
                }
            }
        }
        frames
    }

    /// Hands `msg` to the entry a frame that is not an envelope takes, and
    /// returns the frames it queued.
    async fn top_level_frames(fx: &FoldFixture, msg: TopGunMessage) -> Vec<Vec<u8>> {
        top_level_frames_as(fx, fold_principal(), msg).await
    }

    /// [`top_level_frames`] for a connection authenticated as `principal`.
    async fn top_level_frames_as(
        fx: &FoldFixture,
        principal: Principal,
        msg: TopGunMessage,
    ) -> Vec<Vec<u8>> {
        let (tx, rx) = mpsc::channel(64);
        dispatch_one_message(
            msg,
            fx.conn_id,
            Some(principal),
            &fx.classify_svc,
            &fx.dispatcher,
            &tx,
        )
        .await;
        queued_frames(tx, rx).await
    }

    /// Packs `inner` into one `BATCH` envelope, hands it to the envelope's entry
    /// and returns the frames it queued.
    async fn enveloped_frames(fx: &FoldFixture, inner: &[TopGunMessage]) -> Vec<Vec<u8>> {
        enveloped_frames_as(fx, fold_principal(), inner).await
    }

    /// [`enveloped_frames`] for a connection authenticated as `principal`.
    async fn enveloped_frames_as(
        fx: &FoldFixture,
        principal: Principal,
        inner: &[TopGunMessage],
    ) -> Vec<Vec<u8>> {
        let mut data = Vec::new();
        for msg in inner {
            let bytes = rmp_serde::to_vec_named(msg).expect("inner message serializes");
            data.extend_from_slice(&frame_inner_item(&bytes));
        }
        let batch = BatchMessage {
            count: u32::try_from(inner.len()).expect("inner item count fits in u32"),
            data,
        };
        let (tx, rx) = mpsc::channel(64);
        dispatch_envelope(
            UnpackedEnvelope::unpack(&batch.data),
            fx.conn_id,
            Some(principal),
            &fx.classify_svc,
            &fx.dispatcher,
            &tx,
        )
        .await;
        queued_frames(tx, rx).await
    }

    /// Two keys that hash to two different partitions.
    fn keys_in_two_partitions() -> (String, String) {
        let first = "envelope-key-0".to_string();
        let first_partition = hash_to_partition(&first);
        for i in 1..100_000_u32 {
            let key = format!("envelope-key-{i}");
            if hash_to_partition(&key) != first_partition {
                return (first, key);
            }
        }
        panic!("every candidate key hashed to partition {first_partition}");
    }

    /// One label per frame: the code of an `ERROR` frame, and the whole message
    /// for anything else — so a frame list that should be a single back-off
    /// error reads as `["429"]` and an unexpected frame shows what it was.
    fn error_codes(frames: &[Vec<u8>]) -> Vec<String> {
        decode_frames(frames)
            .iter()
            .map(|msg| match msg {
                TopGunMessage::Error { payload } => payload.code.to_string(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// An `OP_BATCH` inside an envelope reaches the domain service labelled
    /// with its keys' partitions, one sub-batch per partition (TG-DISP-001).
    ///
    /// A batch labelled with no partition runs whole on the worker that serves
    /// unpartitioned work, off the workers its keys belong to — which is what
    /// the split exists to prevent.
    #[tokio::test]
    async fn an_enveloped_op_batch_is_split_onto_its_keys_partition_workers() {
        let (key_a, key_b) = keys_in_two_partitions();
        let (partition_a, partition_b) = (hash_to_partition(&key_a), hash_to_partition(&key_b));
        // Without two partitions a batch that was not split would look the same
        // as one that was.
        assert_ne!(
            partition_a, partition_b,
            "precondition: the two keys are in different partitions"
        );
        let fx = build_fold_fixture(Vec::new(), None).await;
        let ops = vec![
            put_op("1301", FOLD_MAP, &key_a, allowed_fields("first")),
            put_op("1302", FOLD_MAP, &key_b, allowed_fields("second")),
        ];

        enveloped_frames(&fx, &[op_batch_message(ops)]).await;

        let mut expected = vec![Some(partition_a), Some(partition_b)];
        expected.sort_unstable();
        assert_eq!(
            fx.received_partitions(),
            expected,
            "the service must be handed one sub-batch per partition of the batch's keys; \
             (partition, operation count) as received: {:?}",
            fx.received.lock()
        );
    }

    /// A batch with one refused operation is answered with the same bytes inside
    /// an envelope as on its own, and applies the same operations.
    ///
    /// The refusal comes from the real Authorization middleware. It evaluates
    /// whatever it is handed as a unit, so a batch that reaches it whole is
    /// refused whole: nothing is stored and no operation is named.
    #[tokio::test]
    async fn an_enveloped_op_batch_refusal_is_answered_like_a_top_level_one() {
        // One partition, so the frames do not depend on the order two
        // sub-batches happened to finish in.
        let keys = keys_in_one_partition(3);
        let ops = || {
            vec![
                put_op("1401", FOLD_MAP, &keys[0], allowed_fields("first")),
                put_op("1402", FOLD_MAP, &keys[1], blocked_fields()),
                put_op("1403", FOLD_MAP, &keys[2], allowed_fields("third")),
            ]
        };

        let top_level_fx = build_fold_fixture(deny_blocked_records(), None).await;
        let top_level = top_level_frames(&top_level_fx, op_batch_message(ops())).await;
        // An empty top-level answer would make "the frames are equal" true for
        // an envelope that answers nothing.
        let top_level_msgs = decode_frames(&top_level);
        assert_eq!(
            rejections(&top_level_msgs).len(),
            1,
            "precondition: the top-level batch names its refused operation, got {top_level_msgs:?}"
        );
        assert!(
            ack_payloads(&top_level_msgs)
                .iter()
                .any(|ack| ack.results.is_some()),
            "precondition: the top-level batch acknowledges its accepted operations by id, \
             got {top_level_msgs:?}"
        );

        let enveloped_fx = build_fold_fixture(deny_blocked_records(), None).await;
        let enveloped = enveloped_frames(&enveloped_fx, &[op_batch_message(ops())]).await;

        assert_eq!(
            (
                enveloped == top_level,
                enveloped_fx.holds(FOLD_MAP, &keys[0]).await,
                enveloped_fx.holds(FOLD_MAP, &keys[1]).await,
            ),
            (true, true, false),
            "(frames equal to the top-level ones, accepted operation stored, refused operation \
             stored); enveloped frames: {:?}",
            decode_frames(&enveloped)
        );
    }

    /// A message the server is too loaded to take is answered with the back-off
    /// error whether it arrived on its own or inside an envelope (TG-DISP-001).
    ///
    /// The overload is staged at the service because the answer is decided by
    /// the error value, not by who raised it. The single write is the case that
    /// takes the generic classify-and-dispatch route; the batch is answered by
    /// the batch path itself and is here so that neither route goes silent.
    #[tokio::test]
    async fn an_overloaded_inner_message_is_answered_with_a_back_off_frame() {
        let shed_key = "envelope-shed-key".to_string();
        let fx = build_fold_fixture_shedding(Vec::new(), None, Some(shed_key.clone())).await;
        let write = |op_id: &str| put_op(op_id, FOLD_MAP, &shed_key, allowed_fields("shed"));

        let top_level_client_op = top_level_frames(&fx, client_op_message(write("1501"))).await;
        let enveloped_client_op = enveloped_frames(&fx, &[client_op_message(write("1502"))]).await;
        let enveloped_op_batch =
            enveloped_frames(&fx, &[op_batch_message(vec![write("1503")])]).await;

        let back_off = || vec!["429".to_string()];
        assert_eq!(
            (
                error_codes(&top_level_client_op),
                error_codes(&enveloped_client_op),
                error_codes(&enveloped_op_batch),
            ),
            (back_off(), back_off(), back_off()),
            "(top-level CLIENT_OP, enveloped CLIENT_OP, enveloped OP_BATCH): each must be \
             answered with exactly one ERROR frame carrying code 429"
        );
    }

    /// Allow a write to `FOLD_MAP` only when the record names the writer as its
    /// owner, so the same batch is answered differently for two principals.
    fn allow_own_records() -> Vec<PermissionPolicy> {
        vec![PermissionPolicy {
            id: "allow-own-notes".to_string(),
            map_pattern: FOLD_MAP.to_string(),
            action: PermissionAction::Write,
            effect: PolicyEffect::Allow,
            condition: Some(
                parse_permission_expr("auth.id == data.ownerId").expect("owner condition parses"),
            ),
        }]
    }

    /// A record whose owner is `owner_id`.
    fn owned_fields(owner_id: &str) -> Vec<(&'static str, rmpv::Value)> {
        vec![("ownerId", rmpv::Value::String(owner_id.into()))]
    }

    /// A message inside an envelope is authorized as the connection's own
    /// principal, exactly as the same message is on its own (TG-DISP-001).
    ///
    /// The policy admits a write only for the principal its record names, so the
    /// answer to one batch differs between two principals. An envelope that
    /// handed its inner messages on under any other identity — or under none —
    /// would answer at least one of them differently from the top-level path.
    #[tokio::test]
    async fn an_enveloped_message_is_authorized_as_the_connections_principal() {
        let first = fold_principal();
        let second = Principal {
            id: "writer-2".to_string(),
            roles: vec!["user".to_string()],
            tenant: None,
        };
        // One partition, so the frames do not depend on the order two
        // sub-batches happened to finish in.
        let keys = keys_in_one_partition(3);
        let ops = || {
            vec![
                put_op("1701", FOLD_MAP, &keys[0], owned_fields(&first.id)),
                put_op("1702", FOLD_MAP, &keys[1], owned_fields(&second.id)),
                put_op("1703", FOLD_MAP, &keys[2], owned_fields(&first.id)),
            ]
        };

        let top_level_fx_first = build_fold_fixture(allow_own_records(), None).await;
        let top_level_first =
            top_level_frames_as(&top_level_fx_first, first.clone(), op_batch_message(ops())).await;
        let top_level_fx_second = build_fold_fixture(allow_own_records(), None).await;
        let top_level_second = top_level_frames_as(
            &top_level_fx_second,
            second.clone(),
            op_batch_message(ops()),
        )
        .await;
        // If the two principals were answered alike, an envelope that dropped
        // or swapped the principal would be answered alike too.
        let (first_msgs, second_msgs) = (
            decode_frames(&top_level_first),
            decode_frames(&top_level_second),
        );
        assert_eq!(
            (
                rejections(&first_msgs).len(),
                rejections(&second_msgs).len(),
                ack_payloads(&first_msgs).len(),
                ack_payloads(&second_msgs).len(),
                top_level_first == top_level_second,
            ),
            (1, 2, 1, 1, false),
            "precondition: (refusals for the first principal, for the second, acknowledgements \
             for the first, for the second, both answered with the same bytes); \
             first: {first_msgs:?}, second: {second_msgs:?}"
        );

        let enveloped_fx_first = build_fold_fixture(allow_own_records(), None).await;
        let enveloped_first = enveloped_frames_as(
            &enveloped_fx_first,
            first.clone(),
            &[op_batch_message(ops())],
        )
        .await;
        let enveloped_fx_second = build_fold_fixture(allow_own_records(), None).await;
        let enveloped_second = enveloped_frames_as(
            &enveloped_fx_second,
            second.clone(),
            &[op_batch_message(ops())],
        )
        .await;

        assert_eq!(
            (
                enveloped_first == top_level_first,
                enveloped_second == top_level_second,
                enveloped_fx_first.holds(FOLD_MAP, &keys[0]).await,
                enveloped_fx_first.holds(FOLD_MAP, &keys[1]).await,
                enveloped_fx_second.holds(FOLD_MAP, &keys[0]).await,
                enveloped_fx_second.holds(FOLD_MAP, &keys[1]).await,
            ),
            (true, true, true, false, false, true),
            "(first principal's enveloped frames equal its top-level ones, the second's equal \
             its own, first principal: own record stored, the other's record stored, second \
             principal: the other's record stored, own record stored); \
             enveloped as the first: {:?}, enveloped as the second: {:?}",
            decode_frames(&enveloped_first),
            decode_frames(&enveloped_second)
        );
    }

    /// What one path did with two single writes: the frames each was answered
    /// with, whether the first kept the stamp its client sent, and whether the
    /// second was stored.
    type SingleWriteOutcome = (Vec<Vec<u8>>, bool, Vec<Vec<u8>>, bool);

    /// A single write inside an envelope is handled as a write from the
    /// connection it arrived on, exactly as the same write is on its own
    /// (TG-DISP-001).
    ///
    /// The domain service takes a write that names no connection for a trusted
    /// internal call: it keeps the caller's stamp and skips the schema check.
    /// So two effects tell whether the connection was named — a valid write is
    /// stored under a stamp the server minted, and a write the map's schema
    /// rejects is neither acknowledged nor stored.
    #[tokio::test]
    async fn an_enveloped_single_write_is_handled_as_its_connections_write() {
        let valid_key = "envelope-conn-valid";
        let invalid_key = "envelope-conn-invalid";
        let valid = || {
            client_op_message(put_op(
                "1801",
                TYPED_MAP,
                valid_key,
                allowed_fields("named"),
            ))
        };
        // No `name`, which the schema requires.
        let invalid = || {
            client_op_message(put_op(
                "1802",
                TYPED_MAP,
                invalid_key,
                vec![("title", rmpv::Value::String("unnamed".into()))],
            ))
        };
        let client_stamp = match valid() {
            TopGunMessage::ClientOp(msg) => {
                msg.payload
                    .record
                    .flatten()
                    .expect("the write carries a record")
                    .timestamp
            }
            other => panic!("expected a CLIENT_OP, got {other:?}"),
        };
        let schema = || Some((TYPED_MAP, required_name_schema()));

        let top_level_fx = build_fold_fixture(Vec::new(), schema()).await;
        let top_level: SingleWriteOutcome = (
            top_level_frames(&top_level_fx, valid()).await,
            top_level_fx.stored_stamp(TYPED_MAP, valid_key).await == Some(client_stamp.clone()),
            top_level_frames(&top_level_fx, invalid()).await,
            top_level_fx.holds(TYPED_MAP, invalid_key).await,
        );
        // Each element is one the envelope could match by doing nothing, or by
        // doing the trusted thing, unless the top-level path is shown to do the
        // opposite.
        assert_eq!(
            (
                ack_payloads(&decode_frames(&top_level.0)).len(),
                top_level_fx
                    .stored_stamp(TYPED_MAP, valid_key)
                    .await
                    .is_some(),
                top_level.1,
                top_level.2.len(),
                top_level.3,
            ),
            (1, true, false, 0, false),
            "precondition, top-level: (acknowledgements of the valid write, valid write stored, \
             stored under the client's stamp, frames for the schema-invalid write, \
             schema-invalid write stored)"
        );

        let enveloped_fx = build_fold_fixture(Vec::new(), schema()).await;
        let enveloped: SingleWriteOutcome = (
            enveloped_frames(&enveloped_fx, &[valid()]).await,
            enveloped_fx.stored_stamp(TYPED_MAP, valid_key).await == Some(client_stamp),
            enveloped_frames(&enveloped_fx, &[invalid()]).await,
            enveloped_fx.holds(TYPED_MAP, invalid_key).await,
        );

        assert_eq!(
            enveloped,
            top_level,
            "(frames for the valid write, valid write stored under the client's stamp, frames \
             for the schema-invalid write, schema-invalid write stored) — enveloped on the left, \
             top-level on the right; enveloped frames decoded: {:?} and {:?}",
            decode_frames(&enveloped.0),
            decode_frames(&enveloped.2)
        );
    }

    // -----------------------------------------------------------------------
    // What an envelope costs, and that the cost is of the messages dispatched
    // (TG-RATE-001)
    // -----------------------------------------------------------------------

    /// An `OP_BATCH` of `n` default operations, for the tests that only cost a
    /// message and never dispatch it.
    fn costed_op_batch(n: usize) -> TopGunMessage {
        TopGunMessage::OpBatch(OpBatchMessage {
            payload: OpBatchPayload {
                ops: vec![ClientOp::default(); n],
                write_concern: None,
                timeout: None,
            },
        })
    }

    fn costed_ping() -> TopGunMessage {
        TopGunMessage::Ping(topgun_core::messages::PingData { timestamp: 0 })
    }

    /// The bytes of one inner message, before framing.
    fn inner_body(msg: &TopGunMessage) -> Vec<u8> {
        rmp_serde::to_vec_named(msg).expect("inner message serializes")
    }

    /// An envelope whose items are `bodies`, each framed, declaring `count`.
    ///
    /// The declared count is a parameter so a test can make it lie: the
    /// unpacker is handed `data` alone and has no way to read it.
    fn envelope_declaring(count: u32, bodies: &[Vec<u8>]) -> BatchMessage {
        let mut data = Vec::new();
        for body in bodies {
            data.extend_from_slice(&frame_inner_item(body));
        }
        BatchMessage { count, data }
    }

    /// The messages an unpacked envelope holds; none for an over-cap one.
    fn held_messages(envelope: &UnpackedEnvelope) -> &[TopGunMessage] {
        match envelope {
            UnpackedEnvelope::OverCap { .. } => &[],
            UnpackedEnvelope::Unpacked { messages, .. } => messages,
        }
    }

    /// Hands an unpacked envelope to dispatch and returns the frames it queued.
    async fn dispatched_envelope_frames(
        fx: &FoldFixture,
        envelope: UnpackedEnvelope,
    ) -> Vec<Vec<u8>> {
        let (tx, rx) = mpsc::channel(64);
        dispatch_envelope(
            envelope,
            fx.conn_id,
            Some(fold_principal()),
            &fx.classify_svc,
            &fx.dispatcher,
            &tx,
        )
        .await;
        queued_frames(tx, rx).await
    }

    /// `n` storable writes on `FOLD_MAP` with keys `{prefix}-0 .. {prefix}-{n-1}`.
    fn storable_ops(prefix: &str, n: usize) -> Vec<ClientOp> {
        (0..n)
            .map(|i| {
                put_op(
                    &format!("{prefix}-op-{i}"),
                    FOLD_MAP,
                    &format!("{prefix}-{i}"),
                    allowed_fields("costed"),
                )
            })
            .collect()
    }

    /// An `OP_BATCH` inside an envelope costs its operations, and an envelope
    /// costs the sum of what its messages cost.
    #[test]
    fn an_enveloped_op_batch_costs_its_operation_count() {
        let one_batch = envelope_declaring(1, &[inner_body(&costed_op_batch(7))]);
        assert_eq!(
            UnpackedEnvelope::unpack(&one_batch.data).cost(),
            7,
            "an envelope holding one OP_BATCH of 7 operations"
        );

        // 3 + 1 + 1 (an empty batch still costs a token) + 1.
        let mixed = envelope_declaring(
            4,
            &[
                inner_body(&costed_op_batch(3)),
                inner_body(&client_op_message(ClientOp::default())),
                inner_body(&costed_op_batch(0)),
                inner_body(&costed_ping()),
            ],
        );
        assert_eq!(
            UnpackedEnvelope::unpack(&mixed.data).cost(),
            6,
            "an envelope holding OP_BATCH(3), CLIENT_OP, OP_BATCH(0), PING"
        );
    }

    /// One generated item of an envelope.
    #[derive(Debug, Clone)]
    enum GeneratedItem {
        OpBatch(usize),
        ClientOp,
        Ping,
        /// A nested envelope holding one `OP_BATCH` of three operations.
        NestedEnvelope,
        /// A framed item with no body: it does not decode.
        EmptyBody,
        /// The single byte `0xc1`, which `MsgPack` reserves: it does not decode.
        ReservedByte,
    }

    impl GeneratedItem {
        /// The message this item decodes to, if it decodes.
        fn message(&self) -> Option<TopGunMessage> {
            match self {
                Self::OpBatch(n) => Some(costed_op_batch(*n)),
                Self::ClientOp => Some(client_op_message(ClientOp::default())),
                Self::Ping => Some(costed_ping()),
                Self::NestedEnvelope => Some(TopGunMessage::Batch(envelope_declaring(
                    1,
                    &[inner_body(&costed_op_batch(3))],
                ))),
                Self::EmptyBody | Self::ReservedByte => None,
            }
        }

        fn body(&self) -> Vec<u8> {
            match self {
                Self::EmptyBody => Vec::new(),
                Self::ReservedByte => vec![0xc1],
                decodable => inner_body(&decodable.message().expect("the item decodes")),
            }
        }

        /// What the item costs sent on its own; an item that does not decode
        /// costs one token.
        fn expected_cost(&self) -> u32 {
            match self {
                Self::OpBatch(n) => u32::try_from((*n).max(1)).expect("a small count"),
                _ => 1,
            }
        }
    }

    /// Client operations a held message hands to dispatch: a nested envelope
    /// is refused and hands none.
    fn dispatched_units(msg: &TopGunMessage) -> u32 {
        match msg {
            TopGunMessage::OpBatch(batch) => {
                u32::try_from(batch.payload.ops.len()).expect("a small count")
            }
            TopGunMessage::Batch(_) => 0,
            _ => 1,
        }
    }

    /// An envelope costs what its items would cost on their own, holds exactly
    /// the items that decode, in order, and never costs less than the
    /// operations it holds — whatever the header declares and whether or not a
    /// truncated tail follows the last item.
    ///
    /// Run under a fixed generator so a verdict is reproducible.
    #[test]
    fn an_envelope_costs_what_its_messages_cost_on_their_own() {
        use proptest::prelude::*;
        use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

        let item = prop_oneof![
            prop_oneof![Just(0_usize), 1_usize..40].prop_map(GeneratedItem::OpBatch),
            Just(GeneratedItem::ClientOp),
            Just(GeneratedItem::Ping),
            Just(GeneratedItem::NestedEnvelope),
            Just(GeneratedItem::EmptyBody),
            Just(GeneratedItem::ReservedByte),
        ];
        // The tail, when present, is a length prefix cut short or a payload
        // shorter than its prefix says.
        let truncated_tail = proptest::option::of(prop_oneof![
            Just(vec![0_u8, 0]),
            Just(vec![0_u8, 0, 0, 9, 1, 2]),
        ]);
        let case = (
            proptest::collection::vec(item, 1..=12),
            truncated_tail,
            any::<u32>(),
        );

        let mut runner = TestRunner::new_with_rng(
            Config {
                cases: 256,
                failure_persistence: None,
                ..Config::default()
            },
            TestRng::deterministic_rng(RngAlgorithm::ChaCha),
        );
        runner
            .run(&case, |(items, tail, declared_count)| {
                let bodies: Vec<Vec<u8>> = items.iter().map(GeneratedItem::body).collect();
                let mut batch = envelope_declaring(declared_count, &bodies);
                if let Some(tail) = tail {
                    batch.data.extend_from_slice(&tail);
                }

                let envelope = UnpackedEnvelope::unpack(&batch.data);

                let expected_cost: u32 = items.iter().map(GeneratedItem::expected_cost).sum();
                prop_assert_eq!(envelope.cost(), expected_cost.max(1), "cost of {:?}", items);
                let expected_held: Vec<TopGunMessage> =
                    items.iter().filter_map(GeneratedItem::message).collect();
                prop_assert_eq!(
                    held_messages(&envelope),
                    expected_held.as_slice(),
                    "held messages of {:?}",
                    items
                );
                let units: u32 = held_messages(&envelope).iter().map(dispatched_units).sum();
                prop_assert!(
                    units <= envelope.cost(),
                    "{} operations held for a cost of {}: {:?}",
                    units,
                    envelope.cost(),
                    items
                );
                Ok(())
            })
            .expect("every generated envelope costs what its items cost");
    }

    /// The cost an envelope presents is not below the operations dispatch is
    /// handed from it, with every kind of item that could pull the two apart
    /// in one envelope: an empty batch, an item that does not decode, and a
    /// nested envelope.
    ///
    /// The nested `OP_BATCH` holds 7 operations, a size no other figure of the
    /// test has, so a failing tuple says which list was counted.
    #[tokio::test]
    async fn an_envelope_never_presents_a_cost_below_what_it_dispatches() {
        let first = storable_ops("cost-first", 5);
        let last = storable_ops("cost-last", 4);
        let partitions: BTreeSet<u32> = first
            .iter()
            .chain(&last)
            .map(|op| hash_to_partition(&op.key))
            .collect();
        assert!(
            partitions.len() >= 2,
            "precondition: the nine dispatched keys span at least two partitions, got {partitions:?}"
        );
        let nested = TopGunMessage::Batch(envelope_declaring(
            1,
            &[inner_body(&op_batch_message(storable_ops(
                "cost-nested",
                7,
            )))],
        ));
        let bodies = [
            inner_body(&op_batch_message(first)),
            inner_body(&op_batch_message(Vec::new())),
            vec![0xc1],
            inner_body(&nested),
            inner_body(&op_batch_message(last)),
        ];
        // Charged by framed items, this envelope would cost 5; it dispatches 9.
        // Without that gap the test would not tell the two charges apart.
        assert!(
            9 > bodies.len(),
            "precondition: the operations dispatched exceed the framed item count"
        );
        let batch = envelope_declaring(5, &bodies);
        let fx = build_fold_fixture(Vec::new(), None).await;

        let envelope = UnpackedEnvelope::unpack(&batch.data);
        let cost = envelope.cost();
        dispatched_envelope_frames(&fx, envelope).await;

        let dispatched: usize = fx.received.lock().iter().map(|(_, ops)| *ops).sum();
        assert_eq!(
            (cost, dispatched),
            (12, 9),
            "(cost presented, operations handed to the domain service); (partition, operation count) as received: {:?}",
            fx.received.lock()
        );
    }

    /// An envelope inside an envelope costs one token; its contents are neither
    /// costed nor dispatched, and nothing answers it.
    ///
    /// Both headers declare 7, a count equal to nothing the test expects.
    #[tokio::test]
    async fn a_nested_envelope_costs_one_token_and_is_not_unpacked() {
        let nested = TopGunMessage::Batch(envelope_declaring(
            7,
            &[inner_body(&op_batch_message(storable_ops(
                "nested-only",
                9,
            )))],
        ));
        let outer = envelope_declaring(7, &[inner_body(&nested)]);
        let fx = build_fold_fixture(Vec::new(), None).await;

        let envelope = UnpackedEnvelope::unpack(&outer.data);
        let cost = envelope.cost();
        let frames = dispatched_envelope_frames(&fx, envelope).await;

        // Read once and released: a guard still held by the compared tuple
        // would deadlock the failure message's own read of the list.
        let received = fx.received.lock().clone();
        assert_eq!(
            (cost, received.len(), frames.len()),
            (1, 0, 0),
            "(cost, operations handed to the domain service, frames queued); received: {received:?}, frames: {:?}",
            decode_frames(&frames)
        );
    }

    /// An envelope framing more items than the cap is not decoded at all, costs
    /// its framed item count, and is answered whole with one 413.
    ///
    /// The first item is a storable batch: decoded, it would be held, would
    /// change the cost and would reach the domain service.
    #[tokio::test]
    async fn an_oversized_envelope_is_not_decoded_and_is_refused_whole() {
        let mut bodies = vec![inner_body(&op_batch_message(storable_ops("over-cap", 2)))];
        bodies.resize(MAX_BATCH_INNER_ITEMS + 1, Vec::new());
        let batch = envelope_declaring(1, &bodies);
        let fx = build_fold_fixture(Vec::new(), None).await;

        let envelope = UnpackedEnvelope::unpack(&batch.data);
        let (held, cost) = (held_messages(&envelope).len(), envelope.cost());
        let frames = dispatched_envelope_frames(&fx, envelope).await;

        let received = fx.received.lock().len();
        assert_eq!(
            (held, cost, error_codes(&frames), received),
            (0, 8193, vec!["413".to_string()], 0),
            "(messages held, cost, one label per frame queued, operations handed to the domain service)"
        );
    }
}

/// The client writer at every size boundary (`TG-WS-001`): the frames it
/// builds, the bytes it puts on the wire, and how it behaves under control
/// frames and a slow reader.
#[cfg(test)]
mod writer_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::stream::{SplitSink, SplitStream};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::sync::{oneshot, Notify};
    use tokio::task::JoinHandle;

    use super::*;

    /// The fragment length at the default connection settings.
    const F: usize = 393_206;
    /// Every socket buffer in these tests: far below `F`, so the writer meets
    /// back-pressure inside a message.
    const DUPLEX: usize = 64 * 1024;
    const BOUNDARY_LENGTHS: [usize; 15] = [
        0,
        1,
        F - 1,
        F,
        F + 1,
        2 * F,
        2 * F + 1,
        523_264,
        524_278,
        524_279,
        524_288,
        524_289,
        2_097_152,
        2_097_153,
        4_400_000,
    ];

    const OP_CONTINUE: u8 = 0x0;
    const OP_BINARY: u8 = 0x2;
    const OP_CLOSE: u8 = 0x8;
    const OP_PONG: u8 = 0xA;

    /// A payload whose every byte depends on its message and its position, so
    /// a lost, doubled or reordered byte is seen.
    fn payload(message: usize, len: usize) -> Vec<u8> {
        (0..len)
            .map(|at| u8::try_from((at * 31 + message * 7) % 251).expect("below 251"))
            .collect()
    }

    /// `(opcode, fin, payload length)` of each frame.
    fn shapes(frames: &[Frame]) -> Vec<(OpCode, bool, usize)> {
        frames
            .iter()
            .map(|f| (f.header().opcode, f.header().is_final, f.payload().len()))
            .collect()
    }

    /// The frame shapes a message of `len` bytes must have: one `Binary` frame
    /// with `fin` up to `F`, else `Binary` without `fin`, `Continue` frames,
    /// and a last `Continue` with `fin`, each of at most `F` bytes.
    fn expected_shapes(len: usize) -> Vec<(u8, bool, usize)> {
        if len <= F {
            return vec![(OP_BINARY, true, len)];
        }
        let count = len.div_ceil(F);
        (0..count)
            .map(|at| {
                let opcode = if at == 0 { OP_BINARY } else { OP_CONTINUE };
                let fin = at + 1 == count;
                let size = if fin { len - at * F } else { F };
                (opcode, fin, size)
            })
            .collect()
    }

    fn opcode_byte(opcode: OpCode) -> u8 {
        u8::from(opcode)
    }

    #[test]
    fn frame_builder_shapes_at_every_boundary() {
        for (message, len) in BOUNDARY_LENGTHS.into_iter().enumerate() {
            let bytes = payload(message, len);
            let frames: Vec<Frame> = message_frames(Bytes::from(bytes.clone()), F).collect();
            let got: Vec<(u8, bool, usize)> = shapes(&frames)
                .into_iter()
                .map(|(opcode, fin, size)| (opcode_byte(opcode), fin, size))
                .collect();
            assert_eq!(got, expected_shapes(len), "frame shapes for {len} bytes");
            for frame in &frames {
                assert!(frame.payload().len() <= F, "{len}: a frame above F");
                assert!(
                    len == 0 || !frame.payload().is_empty(),
                    "{len}: empty frame"
                );
            }
            let joined: Vec<u8> = frames
                .iter()
                .flat_map(|f| f.payload().iter().copied())
                .collect();
            assert!(joined == bytes, "{len}: payloads do not rejoin the message");
        }
    }

    #[test]
    fn ws_config_carries_every_connection_limit() {
        let custom = ConnectionConfig {
            ws_write_buffer_size: 64 * 1024,
            ws_max_write_buffer_size: 1 << 20,
            ws_max_message_size: 3 << 20,
            ws_max_frame_size: (1 << 20) + 1,
            ..ConnectionConfig::default()
        };
        for connection in [ConnectionConfig::default(), custom] {
            let config = ws_config(&connection);
            assert_eq!(config.write_buffer_size, connection.ws_write_buffer_size);
            assert_eq!(
                config.max_write_buffer_size,
                connection.ws_max_write_buffer_size
            );
            assert_eq!(
                config.max_message_size,
                Some(connection.ws_max_message_size)
            );
            assert_eq!(config.max_frame_size, Some(connection.ws_max_frame_size));
            assert_eq!(config.read_buffer_size, 128 * 1024);
            assert!(!config.accept_unmasked_frames);
        }
    }

    /// One frame as the tap saw it on the server-to-client byte stream.
    #[derive(Debug, Clone)]
    struct WireFrame {
        opcode: u8,
        fin: bool,
        len: usize,
        /// Kept for control frames only (at most 125 bytes).
        control: Vec<u8>,
    }

    /// Parses the frame at the front of `buf` if it is complete: a server
    /// frame, so never masked, with a 7-, 16- or 64-bit length.
    fn parse_frame(buf: &[u8]) -> Option<(WireFrame, usize)> {
        if buf.len() < 2 {
            return None;
        }
        assert_eq!(buf[1] & 0x80, 0, "a server frame is never masked");
        let (len, header) = match buf[1] & 0x7F {
            126 => {
                let ext = buf.get(2..4)?;
                (usize::from(u16::from_be_bytes([ext[0], ext[1]])), 4)
            }
            127 => {
                let ext: [u8; 8] = buf.get(2..10)?.try_into().expect("eight bytes");
                (
                    usize::try_from(u64::from_be_bytes(ext)).expect("fits usize"),
                    10,
                )
            }
            short => (usize::from(short), 2),
        };
        let body = buf.get(header..header + len)?;
        let opcode = buf[0] & 0x0F;
        let frame = WireFrame {
            opcode,
            fin: buf[0] & 0x80 != 0,
            len,
            control: if opcode >= 0x8 {
                body.to_vec()
            } else {
                Vec::new()
            },
        };
        Some((frame, header + len))
    }

    /// A relay between the server's socket and the client's that records
    /// every server frame. With `hold`, it stops reading the server's side
    /// once it has passed on the first fragment of a fragmented message, and
    /// reads on when `release` is notified.
    struct Tap {
        frames: Arc<parking_lot::Mutex<Vec<WireFrame>>>,
        held: oneshot::Receiver<()>,
        release: Arc<Notify>,
    }

    fn tap(server: DuplexStream, client: DuplexStream, hold: bool) -> Tap {
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (mut client_read, mut client_write) = tokio::io::split(client);
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut client_read, &mut server_write).await;
            let _ = server_write.shutdown().await;
        });
        let frames = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let release = Arc::new(Notify::new());
        let (signal_held, on_held) = oneshot::channel();
        let (recorded, released) = (Arc::clone(&frames), Arc::clone(&release));
        tokio::spawn(async move {
            let mut held_tx = hold.then_some(signal_held);
            let mut client_gone = false;
            let mut buf = Vec::new();
            let mut chunk = vec![0u8; 16 * 1024];
            loop {
                while let Some((frame, size)) = parse_frame(&buf) {
                    // Record before forwarding, so the record never depends on
                    // when the client reads.
                    let first_fragment = frame.opcode == OP_BINARY && !frame.fin;
                    recorded.lock().push(frame);
                    // A client that has stopped reading must not stop the
                    // record of what the server sends.
                    if !client_gone && client_write.write_all(&buf[..size]).await.is_err() {
                        client_gone = true;
                    }
                    buf.drain(..size);
                    if first_fragment {
                        if let Some(tx) = held_tx.take() {
                            let _ = tx.send(());
                            released.notified().await;
                        }
                    }
                }
                match server_read.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let _ = client_write.shutdown().await;
        });
        Tap {
            frames,
            held: on_held,
            release,
        }
    }

    type ClientSocket = WebSocketStream<DuplexStream>;

    /// The real writer on a server socket, a reader that drains the server
    /// side as `handle_socket` does, and a client behind a tap.
    struct Harness {
        tx: mpsc::Sender<OutboundMessage>,
        writer: JoinHandle<()>,
        client_sink: SplitSink<ClientSocket, Message>,
        client_stream: SplitStream<ClientSocket>,
        tap: Tap,
    }

    async fn harness(hold: bool) -> Harness {
        harness_with(hold, false).await
    }

    /// The harness on a connection that opted in to chunked delivery when
    /// `chunked`, with the default settings.
    async fn harness_with(hold: bool, chunked: bool) -> Harness {
        let connection = ConnectionConfig::default();
        let (server_io, tap_server) = tokio::io::duplex(DUPLEX);
        let (tap_client, client_io) = tokio::io::duplex(DUPLEX);
        let tap = tap(tap_server, tap_client, hold);
        let server =
            WebSocketStream::from_raw_socket(server_io, Role::Server, Some(ws_config(&connection)))
                .await;
        // A frame above F would fail the client's read, and a message of the
        // largest tested size must fit.
        let client_config = WebSocketConfig::default()
            .max_frame_size(Some(F))
            .max_message_size(Some(8 << 20));
        let client =
            WebSocketStream::from_raw_socket(client_io, Role::Client, Some(client_config)).await;
        let (sink, mut stream) = server.split();
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
        });
        let (tx, rx) = mpsc::channel(256);
        let framing = OutboundFraming::new(&connection, chunked);
        let writer = tokio::spawn(outbound_task(sink, rx, framing));
        let (client_sink, client_stream) = client.split();
        Harness {
            tx,
            writer,
            client_sink,
            client_stream,
            tap,
        }
    }

    /// Reads `count` binary messages, failing on a Close, a read error, or
    /// silence for `within`.
    async fn read_messages(
        stream: &mut SplitStream<ClientSocket>,
        count: usize,
        within: Duration,
    ) -> Vec<Bytes> {
        let mut got = Vec::with_capacity(count);
        while got.len() < count {
            let next = tokio::time::timeout(within, stream.next())
                .await
                .unwrap_or_else(|_| panic!("no message within {within:?} after {}", got.len()))
                .expect("the connection stays open");
            match next.expect("no read error (a frame above F is one)") {
                Message::Binary(bytes) => got.push(bytes),
                Message::Close(frame) => panic!("closed after {} messages: {frame:?}", got.len()),
                _ => {}
            }
        }
        got
    }

    /// Groups the data frames the tap saw into messages, as `(opcode, fin,
    /// len)` per frame.
    fn wire_messages(frames: &[WireFrame]) -> Vec<Vec<(u8, bool, usize)>> {
        let mut messages: Vec<Vec<(u8, bool, usize)>> = Vec::new();
        let mut open = false;
        for frame in frames.iter().filter(|f| f.opcode < 0x8) {
            if !open {
                messages.push(Vec::new());
            }
            messages.last_mut().expect("a message is open").push((
                frame.opcode,
                frame.fin,
                frame.len,
            ));
            open = !frame.fin;
        }
        messages
    }

    #[tokio::test]
    async fn writer_delivers_every_size_whole_in_order() {
        let mut h = harness(false).await;
        let mut lengths = Vec::new();
        for len in BOUNDARY_LENGTHS.into_iter().filter(|&len| len > 0) {
            lengths.push(len);
            lengths.push(16);
        }
        let expected: Vec<Vec<u8>> = lengths
            .iter()
            .enumerate()
            .map(|(message, &len)| payload(message, len))
            .collect();
        let sender = h.tx.clone();
        let to_send = expected.clone();
        tokio::spawn(async move {
            for bytes in to_send {
                sender
                    .send(OutboundMessage::Binary(bytes))
                    .await
                    .expect("queue open");
            }
        });

        let got = read_messages(&mut h.client_stream, lengths.len(), Duration::from_secs(30)).await;
        for (at, (got, want)) in got.iter().zip(&expected).enumerate() {
            assert!(
                got[..] == want[..],
                "message {at} ({} bytes) differs",
                want.len()
            );
        }

        let frames = h.tap.frames.lock().clone();
        let on_wire = wire_messages(&frames);
        let want_wire: Vec<Vec<(u8, bool, usize)>> =
            lengths.iter().map(|&len| expected_shapes(len)).collect();
        assert_eq!(on_wire, want_wire, "frame shapes on the wire");
        assert!(frames.iter().all(|f| f.len <= F), "a frame above F");
        assert!(
            frames.iter().all(|f| f.opcode != OP_CLOSE),
            "the server closed the connection"
        );
        assert!(!h.writer.is_finished(), "the writer ended");
    }

    #[tokio::test]
    async fn writer_answers_ping_between_fragments() {
        let mut h = harness(true).await;
        let message = payload(1, 4_400_000);
        h.tx.send(OutboundMessage::Binary(message.clone()))
            .await
            .expect("queue open");
        let mut client_stream = h.client_stream;
        let reader = tokio::spawn(async move {
            read_messages(&mut client_stream, 1, Duration::from_secs(10)).await
        });
        tokio::time::timeout(Duration::from_secs(10), &mut h.tap.held)
            .await
            .expect("the first fragment within 10 s")
            .expect("tap alive");
        h.client_sink
            .send(Message::Ping(Bytes::from_static(b"mid-message")))
            .await
            .expect("ping sent");
        tokio::time::sleep(Duration::from_millis(100)).await;
        h.tap.release.notify_one();

        let got = reader.await.expect("the client reader does not panic");
        assert!(got[0][..] == message[..], "the message arrived changed");
        let frames = h.tap.frames.lock().clone();
        let pong = frames
            .iter()
            .position(|f| f.opcode == OP_PONG && f.control == b"mid-message")
            .expect("a Pong with the Ping's payload");
        let last = frames
            .iter()
            .position(|f| f.opcode == OP_CONTINUE && f.fin)
            .expect("the message's last frame");
        assert!(
            pong < last,
            "the Pong came after the last frame ({pong} vs {last})"
        );
        assert_eq!(
            frames.iter().filter(|f| f.opcode < 0x8).count(),
            4_400_000_usize.div_ceil(F)
        );
    }

    #[tokio::test]
    async fn writer_stops_cleanly_on_close_between_fragments() {
        let mut h = harness(true).await;
        h.tx.send(OutboundMessage::Binary(payload(1, 4_400_000)))
            .await
            .expect("queue open");
        // Keep the client reading so the tap can pass on what the server sends.
        let mut client_stream = h.client_stream;
        tokio::spawn(async move { while let Some(Ok(_)) = client_stream.next().await {} });
        tokio::time::timeout(Duration::from_secs(10), &mut h.tap.held)
            .await
            .expect("the first fragment within 10 s")
            .expect("tap alive");
        h.client_sink
            .send(Message::Close(None))
            .await
            .expect("close sent");
        tokio::time::sleep(Duration::from_millis(100)).await;
        h.tap.release.notify_one();

        tokio::time::timeout(Duration::from_secs(2), h.writer)
            .await
            .expect("the writer ends within 2 s")
            .expect("the writer does not panic");
        let frames = h.tap.frames.lock().clone();
        let close = frames
            .iter()
            .position(|f| f.opcode == OP_CLOSE)
            .expect("the server answers the Close");
        assert!(
            frames[close..].iter().all(|f| f.opcode >= 0x8),
            "a data frame after the server's Close"
        );
    }

    #[tokio::test]
    async fn writer_pends_on_slow_reader_without_closing() {
        let mut h = harness(false).await;
        let messages = [payload(0, 4_400_000), payload(1, 16), payload(2, 16)];
        for bytes in &messages {
            h.tx.send(OutboundMessage::Binary(bytes.clone()))
                .await
                .expect("queue open");
        }
        // The client reads nothing for 500 ms: the writer must wait on the
        // full socket, not give up on it.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!h.writer.is_finished(), "the writer ended on a slow reader");

        let got = read_messages(&mut h.client_stream, 3, Duration::from_secs(10)).await;
        for (got, want) in got.iter().zip(&messages) {
            assert!(got[..] == want[..], "a message arrived changed");
        }
        assert!(
            h.tap.frames.lock().iter().all(|f| f.opcode != OP_CLOSE),
            "the server closed the connection"
        );
    }

    /// The default effective chunk length.
    const C: usize = 65_536;

    /// Decodes one whole `CHUNK` message, checking its encoding: a map of
    /// exactly `type`, `last` and `data`, in that order, and nothing after it.
    fn decode_chunk(message: &[u8]) -> (bool, Vec<u8>) {
        let mut cursor = std::io::Cursor::new(message);
        let value = rmpv::decode::read_value(&mut cursor).expect("a CHUNK decodes");
        assert_eq!(
            usize::try_from(cursor.position()).expect("fits usize"),
            message.len(),
            "bytes after the CHUNK map"
        );
        let rmpv::Value::Map(entries) = value else {
            panic!("a CHUNK is a map: {value:?}");
        };
        let keys: Vec<&str> = entries
            .iter()
            .map(|(key, _)| key.as_str().expect("a string key"))
            .collect();
        assert_eq!(keys, ["type", "last", "data"], "CHUNK keys and their order");
        assert_eq!(entries[0].1.as_str(), Some("CHUNK"));
        let last = entries[1].1.as_bool().expect("last is a bool");
        let rmpv::Value::Binary(data) = &entries[2].1 else {
            panic!("data is bin: {:?}", entries[2].1);
        };
        (last, data.clone())
    }

    /// Checks the frames `outbound_frames` gives for `input` under a framing
    /// whose chunk length is `chunk_len`, and returns how many `CHUNK`s there
    /// are (0 for the whole-message frames).
    fn check_chunk_frames(input: &Bytes, framing: OutboundFraming) -> usize {
        let chunk_len = framing.chunk_len.expect("a chunked framing");
        let frames: Vec<(Frame, bool)> = outbound_frames(input.clone(), framing).collect();
        let len = input.len();
        if len <= chunk_len {
            let got: Vec<(u8, bool, usize, bool)> = frames
                .iter()
                .map(|(f, flush)| {
                    (
                        opcode_byte(f.header().opcode),
                        f.header().is_final,
                        f.payload().len(),
                        *flush,
                    )
                })
                .collect();
            assert_eq!(got, [(OP_BINARY, true, len, true)], "{len}: whole frame");
            assert!(frames[0].0.payload()[..] == input[..], "{len}: payload");
            return 0;
        }
        assert_eq!(frames.len() % 2, 0, "{len}: every CHUNK is two frames");
        let count = frames.len() / 2;
        assert_eq!(count, len.div_ceil(chunk_len), "{len}: CHUNK count");
        let base = input.as_ptr() as usize;
        let mut joined = Vec::with_capacity(len);
        for (at, pair) in frames.chunks(2).enumerate() {
            let ((header, header_flush), (data, data_flush)) = (&pair[0], &pair[1]);
            assert_eq!(opcode_byte(header.header().opcode), OP_BINARY);
            assert!(!header.header().is_final, "{len}: header frame with fin");
            assert!(!header_flush, "{len}: the header frame flushes alone");
            assert!(header.payload().len() <= CHUNK_HEADER_MAX, "{len}: header");
            assert!(header.payload().len() <= framing.fragment_len);
            assert_eq!(opcode_byte(data.header().opcode), OP_CONTINUE);
            assert!(data.header().is_final, "{len}: data frame without fin");
            assert!(data_flush, "{len}: the data frame does not flush");
            assert!(!data.payload().is_empty(), "{len}: empty CHUNK {at}");
            assert!(data.payload().len() <= chunk_len, "{len}: CHUNK above C");
            assert!(data.payload().len() <= framing.fragment_len);
            let at_ptr = data.payload().as_ptr() as usize;
            assert!(
                at_ptr >= base && at_ptr + data.payload().len() <= base + len,
                "{len}: CHUNK {at} is a copy, not a view of the message"
            );
            let mut message = header.payload().to_vec();
            message.extend_from_slice(data.payload());
            let (last, bytes) = decode_chunk(&message);
            assert_eq!(last, at + 1 == count, "{len}: last on CHUNK {at}");
            joined.extend_from_slice(&bytes);
        }
        assert!(joined[..] == input[..], "{len}: CHUNKs do not rejoin");
        count
    }

    #[test]
    fn chunk_capability_needs_the_exact_pair() {
        let cases = [
            (None, false),
            (Some(""), false),
            (Some("tg_chunks=1"), true),
            (Some("a=b&tg_chunks=1"), true),
            (Some("tg_chunks=1&a=b"), true),
            (Some("tg_chunks=0"), false),
            (Some("tg_chunks=1x"), false),
            (Some("tg_chunks"), false),
            (Some("TG_CHUNKS=1"), false),
            (Some("xtg_chunks=1"), false),
        ];
        for (query, want) in cases {
            assert_eq!(chunk_capability(query), want, "{query:?}");
        }
    }

    #[test]
    fn chunk_builder_shapes_at_every_boundary() {
        for chunk_len in [C, 16] {
            let framing = OutboundFraming {
                fragment_len: F,
                chunk_len: Some(chunk_len),
            };
            let lengths = [
                0,
                1,
                chunk_len - 1,
                chunk_len,
                chunk_len + 1,
                2 * chunk_len - 1,
                2 * chunk_len,
                2 * chunk_len + 1,
                4_400_000,
            ];
            for (message, len) in lengths.into_iter().enumerate() {
                let input = Bytes::from(payload(message, len));
                let count = check_chunk_frames(&input, framing);
                let want = if len <= chunk_len {
                    0
                } else {
                    len.div_ceil(chunk_len)
                };
                assert_eq!(count, want, "C = {chunk_len}, {len} bytes");
            }
        }
    }

    #[test]
    fn outbound_framing_clamps_the_chunk_length() {
        let default = ConnectionConfig::default();
        let f = default.outbound_fragment_len();
        assert_eq!(
            OutboundFraming::new(&default, false),
            OutboundFraming {
                fragment_len: f,
                chunk_len: None
            }
        );
        assert_eq!(OutboundFraming::new(&default, true).chunk_len, Some(C));

        let zero = ConnectionConfig {
            outbound_chunk_len: 0,
            ..ConnectionConfig::default()
        };
        let framing = OutboundFraming::new(&zero, true);
        assert_eq!(framing.chunk_len, Some(1), "a chunk length of 0");
        assert_eq!(check_chunk_frames(&Bytes::from_static(b"abc"), framing), 3);

        for configured in [f, usize::MAX] {
            let oversized = ConnectionConfig {
                outbound_chunk_len: configured,
                ..ConnectionConfig::default()
            };
            let framing = OutboundFraming::new(&oversized, true);
            assert_eq!(
                framing.chunk_len,
                Some(f - CHUNK_HEADER_MAX),
                "{configured}"
            );
            let input = Bytes::from(payload(3, 4_400_000));
            assert_eq!(
                check_chunk_frames(&input, framing),
                4_400_000_usize.div_ceil(f - CHUNK_HEADER_MAX)
            );
        }

        let tight = |budget: usize| ConnectionConfig {
            ws_write_buffer_size: 1_000,
            ws_max_write_buffer_size: 1_010 + budget,
            ..ConnectionConfig::default()
        };
        let thirty_two = tight(32);
        assert_eq!(thirty_two.outbound_fragment_len(), 32);
        let framing = OutboundFraming::new(&thirty_two, true);
        assert_eq!(framing.chunk_len, None, "no CHUNK fits a 32-byte frame");
        let frames: Vec<(Frame, bool)> =
            outbound_frames(Bytes::from(payload(4, 4_400_000)), framing).collect();
        assert_eq!(frames.len(), 4_400_000_usize.div_ceil(32));
        assert!(frames
            .iter()
            .all(|(f, flush)| *flush && f.payload().len() == 32));
        assert_eq!(opcode_byte(frames[0].0.header().opcode), OP_BINARY);
        assert!(frames[1..]
            .iter()
            .all(|(f, _)| opcode_byte(f.header().opcode) == OP_CONTINUE));

        let thirty_three = tight(33);
        assert_eq!(thirty_three.outbound_fragment_len(), 33);
        assert_eq!(OutboundFraming::new(&thirty_three, true).chunk_len, Some(1));
    }

    /// Reads messages until the server's Close, failing on a read error or
    /// silence for `within`.
    async fn read_until_close(
        stream: &mut SplitStream<ClientSocket>,
        within: Duration,
    ) -> Vec<Bytes> {
        let mut got = Vec::new();
        loop {
            let next = tokio::time::timeout(within, stream.next())
                .await
                .unwrap_or_else(|_| panic!("no message within {within:?} after {}", got.len()));
            match next {
                Some(Ok(Message::Binary(bytes))) => got.push(bytes),
                Some(Ok(Message::Close(_))) | None => return got,
                Some(Ok(_)) => {}
                Some(Err(e)) => panic!("read error after {} messages: {e}", got.len()),
            }
        }
    }

    /// The `CHUNK`s of `messages`, which must rejoin `want`.
    fn rejoin_chunks(messages: &[Bytes], want: &[u8]) {
        let mut joined = Vec::with_capacity(want.len());
        for (at, message) in messages.iter().enumerate() {
            let (last, data) = decode_chunk(message);
            assert_eq!(last, at + 1 == messages.len(), "last on CHUNK {at}");
            assert!(!data.is_empty(), "empty CHUNK {at}");
            joined.extend_from_slice(&data);
        }
        assert!(
            joined[..] == want[..],
            "the CHUNKs do not rejoin the message"
        );
    }

    #[tokio::test]
    async fn chunked_writer_sends_every_chunk_before_the_next_message() {
        let mut h = harness_with(false, true).await;
        let big = payload(1, 4_400_000);
        let small = payload(2, 10);
        for bytes in [&big, &small] {
            h.tx.send(OutboundMessage::Binary(bytes.clone()))
                .await
                .expect("queue open");
        }
        let k = 4_400_000_usize.div_ceil(C);
        let got = read_messages(&mut h.client_stream, k + 1, Duration::from_secs(30)).await;
        rejoin_chunks(&got[..k], &big);
        assert!(got[k][..] == small[..], "the small message arrived changed");

        let frames = h.tap.frames.lock().clone();
        let on_wire = wire_messages(&frames);
        assert_eq!(on_wire.len(), k + 1, "WebSocket messages on the wire");
        for (at, message) in on_wire[..k].iter().enumerate() {
            let data = if at + 1 == k { 4_400_000 - at * C } else { C };
            assert_eq!(message.len(), 2, "CHUNK {at} is two frames");
            assert_eq!((message[0].0, message[0].1), (OP_BINARY, false));
            assert!(message[0].2 <= CHUNK_HEADER_MAX);
            assert_eq!(message[1], (OP_CONTINUE, true, data), "CHUNK {at}");
        }
        assert_eq!(on_wire[k], expected_shapes(10));
        assert!(frames.iter().all(|f| f.opcode != OP_CLOSE), "closed");
        assert!(!h.writer.is_finished(), "the writer ended");
    }

    #[tokio::test]
    async fn chunked_writer_sends_a_queued_close_after_the_last_chunk() {
        let mut h = harness_with(false, true).await;
        let big = payload(1, 4_400_000);
        h.tx.send(OutboundMessage::Binary(big.clone()))
            .await
            .expect("queue open");
        h.tx.send(OutboundMessage::Close(None))
            .await
            .expect("queue open");
        let got = read_until_close(&mut h.client_stream, Duration::from_secs(30)).await;
        assert_eq!(
            got.len(),
            4_400_000_usize.div_ceil(C),
            "CHUNKs before the Close"
        );
        rejoin_chunks(&got, &big);
        let frames = h.tap.frames.lock().clone();
        let close = frames
            .iter()
            .position(|f| f.opcode == OP_CLOSE)
            .expect("the server's Close");
        assert!(
            frames[close..].iter().all(|f| f.opcode >= 0x8),
            "a data frame after the Close"
        );
    }

    #[tokio::test]
    async fn chunked_writer_answers_ping_between_chunks() {
        let mut h = harness_with(true, true).await;
        let big = payload(1, 4_400_000);
        h.tx.send(OutboundMessage::Binary(big.clone()))
            .await
            .expect("queue open");
        let k = 4_400_000_usize.div_ceil(C);
        let mut client_stream = h.client_stream;
        let reader = tokio::spawn(async move {
            read_messages(&mut client_stream, k, Duration::from_secs(10)).await
        });
        tokio::time::timeout(Duration::from_secs(10), &mut h.tap.held)
            .await
            .expect("the first CHUNK within 10 s")
            .expect("tap alive");
        h.client_sink
            .send(Message::Ping(Bytes::from_static(b"mid-chunks")))
            .await
            .expect("ping sent");
        tokio::time::sleep(Duration::from_millis(100)).await;
        h.tap.release.notify_one();

        let got = reader.await.expect("the client reader does not panic");
        rejoin_chunks(&got, &big);
        let frames = h.tap.frames.lock().clone();
        let pong = frames
            .iter()
            .position(|f| f.opcode == OP_PONG && f.control == b"mid-chunks")
            .expect("a Pong with the Ping's payload");
        let last = frames
            .iter()
            .rposition(|f| f.opcode == OP_CONTINUE && f.fin)
            .expect("the last CHUNK's data frame");
        assert!(
            pong < last,
            "the Pong came after the last CHUNK ({pong} vs {last})"
        );
    }

    #[tokio::test]
    async fn whole_message_writer_keeps_the_legacy_frames() {
        let mut h = harness(false).await;
        let big = payload(1, 4_400_000);
        let small = payload(2, 10);
        for bytes in [&big, &small] {
            h.tx.send(OutboundMessage::Binary(bytes.clone()))
                .await
                .expect("queue open");
        }
        h.tx.send(OutboundMessage::Close(None))
            .await
            .expect("queue open");
        let got = read_until_close(&mut h.client_stream, Duration::from_secs(30)).await;
        assert_eq!(got.len(), 2, "whole messages, no CHUNK");
        assert!(got[0][..] == big[..] && got[1][..] == small[..]);
        let frames = h.tap.frames.lock().clone();
        assert_eq!(
            wire_messages(&frames),
            vec![expected_shapes(4_400_000), expected_shapes(10)]
        );
    }
}
