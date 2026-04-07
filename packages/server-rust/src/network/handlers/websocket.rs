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

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::State;
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use tokio::sync::mpsc;
use topgun_core::messages::{
    AuthAckData, ErrorPayload, OpAckMessage, OpAckPayload, Message as TopGunMessage, WriteConcern,
};
use topgun_core::hash_to_partition;
use tracing::{debug, warn};

use super::auth::AuthHandler;
use super::AppState;
use crate::network::connection::ConnectionId;
use crate::network::{ConnectionKind, OutboundMessage};
use crate::service::classify::OperationService;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::operation::{CallerOrigin, ClassifyError, OperationError, OperationResponse};
use topgun_core::Principal;

/// Maximum number of in-flight dispatch tasks per connection.
///
/// Each spawned task holds a semaphore permit until its dispatch completes.
/// This bounds memory and task overhead: at 6-11µs per op, 32 slots is
/// ~200µs of parallelism — enough to saturate the pipeline without
/// accumulating an unbounded backlog.
const MAX_IN_FLIGHT: usize = 32;

/// Upgrades an HTTP connection to a WebSocket connection.
///
/// Configures write buffer sizes from the connection config, then hands
/// off to `handle_socket` for the message processing loops.
pub async fn ws_upgrade_handler(
    State(state): State<AppState>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    ws.write_buffer_size(state.config.connection.ws_write_buffer_size)
        .max_write_buffer_size(state.config.connection.ws_max_write_buffer_size)
        .on_upgrade(|socket| handle_socket(socket, state))
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
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let (handle, rx) = state
        .registry
        .register(ConnectionKind::Client, &state.config.connection);
    let conn_id = handle.id;

    debug!("WebSocket connected: {:?}", conn_id);

    // Send AUTH_REQUIRED before splitting the socket, so the client
    // knows to authenticate before sending any other messages.
    if let Some(ref secret) = state.jwt_secret {
        let auth_handler = AuthHandler::new(secret.clone(), state.auth_validator.clone());
        if let Err(e) = auth_handler.send_auth_required(&mut socket).await {
            warn!(
                "failed to send AUTH_REQUIRED to {:?}: {}",
                conn_id, e
            );
            state.registry.remove(conn_id);
            return;
        }
    }

    let (sender, mut receiver) = socket.split();

    // Outbound task owns the write half of the socket and drains
    // the mpsc channel, coalescing messages when multiple are ready.
    let outbound_handle = tokio::spawn(outbound_task(sender, rx));

    // Tracks auth state for phase switching. AtomicBool avoids RwLock
    // contention in Phase 2 — set once in Phase 1, read once to decide
    // which phase to enter. handle.metadata is still written during Phase 1
    // so domain services can read the principal.
    let authenticated = AtomicBool::new(false);

    // Semaphore limits in-flight dispatch tasks to MAX_IN_FLIGHT.
    // Closed on shutdown to unblock any pending acquire.
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));

    // Phase 1: sequential auth — only proceed when JWT secret is configured.
    // If no secret is set, every connection is pre-authenticated.
    if state.jwt_secret.is_some() {
        'auth: loop {
            match receiver.next().await {
                Some(Ok(Message::Binary(data))) => {
                    let tg_msg = match rmp_serde::from_slice::<TopGunMessage>(&data) {
                        Ok(msg) => msg,
                        Err(e) => {
                            debug!(
                                "failed to deserialize message from {:?}: {}",
                                conn_id, e
                            );
                            continue;
                        }
                    };

                    if let TopGunMessage::Auth(ref auth_msg) = tg_msg {
                        if let Some(ref secret) = state.jwt_secret {
                            let auth_handler = AuthHandler::new(secret.clone(), state.auth_validator.clone());
                            match auth_handler.handle_auth(auth_msg, &handle.tx, state.config.jwt_clock_skew_secs, state.config.insecure_forward_auth_errors).await {
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

                                    // Send AUTH_ACK with userId via the outbound channel
                                    let ack_msg = TopGunMessage::AuthAck(AuthAckData {
                                        user_id: Some(principal.id.clone()),
                                        ..Default::default()
                                    });
                                    if let Ok(bytes) = rmp_serde::to_vec_named(&ack_msg) {
                                        let _ = handle.tx.send(OutboundMessage::Binary(bytes)).await;
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
                                    debug!(
                                        "auth failed for {:?}: {}",
                                        conn_id, e
                                    );
                                    // Drain semaphore and drop before returning
                                    semaphore.close();
                                    drop(handle);
                                    tokio::time::timeout(
                                        std::time::Duration::from_secs(2),
                                        outbound_handle,
                                    )
                                    .await
                                    .ok();
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
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        outbound_handle,
                    )
                    .await
                    .ok();
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
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                    // Handled automatically by axum/tungstenite.
                }
                Some(Err(e)) => {
                    debug!(
                        "WebSocket error on connection {:?} during auth: {}",
                        conn_id, e
                    );
                    semaphore.close();
                    drop(handle);
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        outbound_handle,
                    )
                    .await
                    .ok();
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
    let principal: Option<Principal> = {
        let meta = handle.metadata.read().await;
        meta.principal.clone()
    };

    // Phase 2: pipeline mode — each binary frame is dispatched concurrently.
    // The reader continues immediately after spawning, so multiple frames
    // can be in-flight simultaneously up to MAX_IN_FLIGHT.
    loop {
        match receiver.next().await {
            Some(Ok(Message::Binary(data))) => {
                let tg_msg = match rmp_serde::from_slice::<TopGunMessage>(&data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        debug!(
                            "failed to deserialize message from {:?}: {}",
                            conn_id, e
                        );
                        continue;
                    }
                };

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
                    dispatch_message(tg_msg, conn_id, principal_clone, op_service, dispatcher, tx).await;
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
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                // Ping/Pong are handled automatically by axum/tungstenite.
            }
            Some(Err(e)) => {
                debug!(
                    "WebSocket error on connection {:?}: {}",
                    conn_id, e
                );
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

    // Wait for the outbound task to finish flushing, with a timeout to
    // prevent hanging if the writer is stuck.
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        outbound_handle,
    )
    .await
    .ok();

    state.registry.remove(conn_id);
    debug!("WebSocket disconnected: {:?}", conn_id);
}

/// Dispatches a single deserialized message through the operation pipeline.
///
/// Takes owned Arc and Sender so this function can be moved into a
/// `tokio::spawn` closure (satisfying the `'static` bound). Helpers called
/// from within this function borrow from its owned locals.
///
/// Handles BATCH messages by unpacking and routing each inner message
/// individually. Non-BATCH messages are classified, have `connection_id`
/// and `principal` set, and are routed through the pipeline. Each
/// `OperationResponse` variant is mapped to the appropriate outbound message(s).
async fn dispatch_message(
    tg_msg: TopGunMessage,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    operation_service: Option<Arc<OperationService>>,
    dispatcher: Option<Arc<PartitionDispatcher>>,
    tx: mpsc::Sender<OutboundMessage>,
) {
    let (Some(classify_svc), Some(dispatcher)) = (operation_service, dispatcher) else {
        debug!("dispatcher not configured, dropping message from {:?}", conn_id);
        return;
    };

    // Handle BATCH messages: unpack each inner message and route individually
    if let TopGunMessage::Batch(ref batch_msg) = tg_msg {
        unpack_and_dispatch_batch(batch_msg, conn_id, principal, &classify_svc, &dispatcher, &tx).await;
        return;
    }

    // Intercept OpBatch messages before generic classify/dispatch.
    // Split by partition so each sub-batch runs on a dedicated partition worker
    // rather than serializing all ops on the single global worker.
    if let TopGunMessage::OpBatch(ref batch_msg) = tg_msg {
        dispatch_op_batch(batch_msg, conn_id, principal, &classify_svc, &dispatcher, &tx).await;
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
                    send_operation_response(resp, &tx).await;
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
            // BATCH messages should be caught above; log if another envelope type appears
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
    }
}

/// Splits an `OpBatch` by partition and dispatches all sub-batches concurrently.
///
/// Groups the batch's ops by `hash_to_partition(key)`, creates one
/// `Operation::OpBatch` per partition group (each carrying `partition_id=Some(id)`
/// so the dispatcher routes it to the correct partition worker instead of the
/// single global worker), dispatches all groups concurrently, and sends a single
/// `OP_ACK` with `lastId` from the last op in the original batch.
///
/// Per-sub-batch `OpAck` responses from `CrdtService::handle_op_batch()` are
/// discarded; the aggregated ack is constructed from the original batch's
/// last-op ID so the client always receives exactly one `OP_ACK`.
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
        let ack = TopGunMessage::OpAck(OpAckMessage {
            payload: OpAckPayload {
                last_id: "unknown".to_string(),
                ..Default::default()
            },
        });
        if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
            let _ = tx.send(OutboundMessage::Binary(bytes)).await;
        }
        return;
    }

    // Compute lastId from the last op in the original batch order.
    let last_id = ops
        .last()
        .and_then(|op| op.id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Group ops by their partition so each group targets one partition worker.
    let mut partition_groups: HashMap<u32, Vec<topgun_core::messages::ClientOp>> = HashMap::new();
    for op in ops {
        let partition_id = hash_to_partition(&op.key);
        partition_groups.entry(partition_id).or_default().push(op.clone());
    }

    let write_concern = batch_msg.payload.write_concern.clone();
    let timeout = batch_msg.payload.timeout;

    // Build all sub-batch operations up front, then dispatch concurrently.
    let mut sub_ops: Vec<crate::service::operation::Operation> =
        Vec::with_capacity(partition_groups.len());
    for (partition_id, group_ops) in partition_groups {
        let mut op = classify_svc.classify_op_batch_for_partition(
            group_ops,
            partition_id,
            None,
            CallerOrigin::Client,
            write_concern.clone(),
            timeout,
        );
        op.set_connection_id(conn_id);
        if let Some(p) = principal.clone() {
            op.set_principal(p);
        }
        sub_ops.push(op);
    }

    // Dispatch all sub-batches concurrently; collect results.
    let mut join_set = tokio::task::JoinSet::new();
    for sub_op in sub_ops {
        let dispatcher = Arc::clone(dispatcher);
        join_set.spawn(async move { dispatcher.dispatch(sub_op).await });
    }

    // Collect results and check for errors. Per-sub-batch OpAck responses are
    // discarded; the aggregated OP_ACK is built from the original batch's lastId.
    // Preserve the OperationError type so we can distinguish 429 from 500
    // without inspecting string content.
    let mut dispatch_error: Option<OperationError> = None;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_resp)) => {
                // Discard the per-sub-batch OpAck; we send one aggregated ack below.
            }
            Ok(Err(e)) => {
                dispatch_error = Some(e);
            }
            Err(join_err) => {
                dispatch_error = Some(OperationError::Internal(anyhow::anyhow!(
                    "join error: {join_err}"
                )));
            }
        }
    }

    if let Some(err) = dispatch_error {
        debug!("dispatch_op_batch error for {:?}: {}", conn_id, err);
        let (code, message) = match err {
            OperationError::Overloaded => {
                (429, "server overloaded, try again later".to_string())
            }
            ref e => (500, format!("{e}")),
        };
        let err_response = TopGunMessage::Error {
            payload: ErrorPayload {
                code,
                message,
                details: None,
            },
        };
        if let Ok(bytes) = rmp_serde::to_vec_named(&err_response) {
            let _ = tx.send(OutboundMessage::Binary(bytes)).await;
        }
        return;
    }

    // All sub-batches succeeded — send one OP_ACK with the original batch's lastId.
    // Sub-batch responses are discarded; set APPLIED explicitly on the aggregated ack
    // because each sub-batch's CRDT merge succeeded in memory.
    let ack = TopGunMessage::OpAck(OpAckMessage {
        payload: OpAckPayload {
            last_id,
            achieved_level: Some(WriteConcern::APPLIED),
            ..Default::default()
        },
    });
    if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
        let _ = tx.send(OutboundMessage::Binary(bytes)).await;
    }
}

/// Unpacks a BATCH message and dispatches each inner message individually.
///
/// The BATCH `data` field contains length-prefixed binary messages: each
/// inner message is preceded by a 4-byte big-endian u32 length header.
async fn unpack_and_dispatch_batch(
    batch_msg: &topgun_core::messages::BatchMessage,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    classify_svc: &OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    tx: &mpsc::Sender<OutboundMessage>,
) {
    let data = &batch_msg.data;
    let mut offset = 0;

    while offset < data.len() {
        // Read 4-byte big-endian length prefix
        if offset + 4 > data.len() {
            debug!("truncated batch length prefix from {:?}", conn_id);
            break;
        }
        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        if offset + len > data.len() {
            debug!(
                "truncated batch message (need {} bytes, {} available) from {:?}",
                len,
                data.len() - offset,
                conn_id
            );
            break;
        }

        let msg_bytes = &data[offset..offset + len];
        offset += len;

        // Deserialize the inner message
        let inner_msg = match rmp_serde::from_slice::<TopGunMessage>(msg_bytes) {
            Ok(msg) => msg,
            Err(e) => {
                debug!(
                    "failed to deserialize batch inner message from {:?}: {}",
                    conn_id, e
                );
                continue;
            }
        };

        // Classify and route each inner message individually.
        // Inner messages target different services and partitions, so each
        // must be dispatched separately for correct partition routing.
        match classify_svc.classify(inner_msg, None, CallerOrigin::Client) {
            Ok(mut op) => {
                op.set_connection_id(conn_id);
                if let Some(p) = principal.clone() {
                    op.set_principal(p);
                }

                match dispatcher.dispatch(op).await {
                    Ok(resp) => {
                        send_operation_response(resp, tx).await;
                    }
                    Err(e) => {
                        debug!("dispatch error for batch item from {:?}: {}", conn_id, e);
                    }
                }
            }
            Err(e) => {
                debug!(
                    "failed to classify batch inner message from {:?}: {}",
                    conn_id, e
                );
            }
        }
    }
}

/// Sends an `OperationResponse` as outbound WebSocket message(s).
///
/// Maps each variant to the appropriate wire format:
/// - `Message` -> serialize and send as binary frame
/// - `Messages` -> serialize each individually and send as separate frames
/// - `Empty` -> no response
/// - `Ack` -> construct `OpAck` with `call_id.to_string()` as `last_id`
/// - `NotImplemented` -> construct `Error` with code 501
async fn send_operation_response(
    resp: OperationResponse,
    tx: &mpsc::Sender<OutboundMessage>,
) {
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
        OperationResponse::Ack { call_id } => {
            let ack = TopGunMessage::OpAck(OpAckMessage {
                payload: OpAckPayload {
                    last_id: call_id.to_string(),
                    ..Default::default()
                },
            });
            if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
                let _ = tx.send(OutboundMessage::Binary(bytes)).await;
            }
        }
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

/// Drains the outbound mpsc channel and writes messages to the WebSocket.
///
/// Implements message coalescing: after receiving the first message, it
/// checks `try_recv()` for additional ready messages and sends them all
/// before waiting again. This reduces the number of individual write
/// syscalls under load.
async fn outbound_task(
    mut sender: SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<OutboundMessage>,
) {
    while let Some(msg) = rx.recv().await {
        if send_outbound_message(&mut sender, msg).await.is_err() {
            break;
        }

        // Coalesce: drain any additional messages that are already buffered
        // in the channel without blocking. This batches multiple messages
        // into a burst of writes before flushing.
        while let Ok(msg) = rx.try_recv() {
            if send_outbound_message(&mut sender, msg).await.is_err() {
                return;
            }
        }

        // Flush after draining all ready messages to push the batch to
        // the network in a single write.
        if sender.flush().await.is_err() {
            break;
        }
    }

    // Gracefully close the WebSocket write half.
    let _ = sender.close().await;
}

/// Sends a single outbound message to the WebSocket sender.
///
/// Returns `Ok(())` on success or `Err(())` when the connection should
/// be torn down (send error or Close message).
async fn send_outbound_message(
    sender: &mut SplitSink<WebSocket, Message>,
    msg: OutboundMessage,
) -> Result<(), ()> {
    match msg {
        OutboundMessage::Binary(data) => {
            if sender.send(Message::Binary(data.into())).await.is_err() {
                return Err(());
            }
        }
        OutboundMessage::Close(reason) => {
            let close_frame = reason.map(|r| CloseFrame {
                code: axum::extract::ws::close_code::NORMAL,
                reason: r.into(),
            });
            let _ = sender.send(Message::Close(close_frame)).await;
            return Err(());
        }
    }
    Ok(())
}
