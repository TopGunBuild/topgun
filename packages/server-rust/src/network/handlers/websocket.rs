//! WebSocket upgrade handler with inbound/outbound message loops.
//!
//! Uses the socket-split pattern: the WebSocket is split into a sender
//! (owned by the outbound task) and a receiver (owned by the inbound loop).
//! This avoids holding a single mutable reference across concurrent reads
//! and writes.
//!
//! The authentication gate ensures unauthenticated connections can only
//! send AUTH messages; all other message types are silently dropped until
//! the connection is authenticated.

use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::State;
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use tokio::sync::mpsc;
use topgun_core::messages::{
    AuthAckData, ErrorPayload, OpAckMessage, OpAckPayload, Message as TopGunMessage,
};
use tower::{Service, ServiceExt};
use tracing::{debug, warn};

use super::auth::AuthHandler;
use super::AppState;
use crate::network::connection::ConnectionId;
use crate::network::{ConnectionKind, OutboundMessage};
use crate::service::classify::OperationService;
use crate::service::operation::{
    CallerOrigin, ClassifyError, OperationPipeline, OperationResponse,
};

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
/// The function sends AUTH_REQUIRED before splitting, then splits the socket
/// into sender/receiver halves, spawns an outbound task for writes, and
/// runs the inbound loop in the current task. Unauthenticated connections
/// can only send AUTH messages; all others are dropped. When the inbound
/// loop exits (client disconnect or error), the connection is removed from
/// the registry.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let (handle, rx) = state
        .registry
        .register(ConnectionKind::Client, &state.config.connection);
    let conn_id = handle.id;

    debug!("WebSocket connected: {:?}", conn_id);

    // Send AUTH_REQUIRED before splitting the socket, so the client
    // knows to authenticate before sending any other messages.
    if let Some(ref secret) = state.jwt_secret {
        let auth_handler = AuthHandler::new(secret.clone());
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

    // Inbound loop: read messages from the client until disconnect or error.
    while let Some(result) = receiver.next().await {
        match result {
            Ok(Message::Binary(data)) => {
                // Deserialize inbound MsgPack binary into a TopGun message
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

                // Check authentication state
                let is_authenticated = {
                    let meta = handle.metadata.read().await;
                    meta.authenticated
                };

                if !is_authenticated {
                    // Only AUTH messages are allowed before authentication
                    if let TopGunMessage::Auth(ref auth_msg) = tg_msg {
                        if let Some(ref secret) = state.jwt_secret {
                            let auth_handler = AuthHandler::new(secret.clone());
                            match auth_handler.handle_auth(auth_msg, &handle.tx).await {
                                Ok(principal) => {
                                    // Update connection metadata with auth state
                                    {
                                        let mut meta = handle.metadata.write().await;
                                        meta.authenticated = true;
                                        meta.principal = Some(principal.clone());
                                    }

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
                                }
                                Err(e) => {
                                    // AUTH_FAIL already sent by handle_auth; close connection
                                    debug!(
                                        "auth failed for {:?}: {}",
                                        conn_id, e
                                    );
                                    break;
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
                } else {
                    // Authenticated: dispatch through operation pipeline
                    dispatch_message(
                        tg_msg,
                        conn_id,
                        &state.operation_service,
                        &state.operation_pipeline,
                        &handle.tx,
                    )
                    .await;
                }
            }
            Ok(Message::Close(_)) => {
                debug!("close frame received from connection {:?}", conn_id);
                break;
            }
            Ok(Message::Text(_)) => {
                // TopGun uses binary MsgPack only; text messages are unexpected.
                warn!(
                    "ignoring text message from connection {:?} -- binary only",
                    conn_id
                );
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => {
                // Ping/Pong are handled automatically by axum/tungstenite.
            }
            Err(e) => {
                debug!(
                    "WebSocket error on connection {:?}: {}",
                    conn_id, e
                );
                break;
            }
        }
    }

    // Clean up: abort the outbound task and remove from registry.
    outbound_handle.abort();
    state.registry.remove(conn_id);
    debug!("WebSocket disconnected: {:?}", conn_id);
}

/// Dispatches a single deserialized message through the operation pipeline.
///
/// Handles BATCH messages by unpacking and routing each inner message
/// individually. Non-BATCH messages are classified, have `connection_id`
/// set, and are routed through the pipeline. Each `OperationResponse`
/// variant is mapped to the appropriate outbound message(s).
async fn dispatch_message(
    tg_msg: TopGunMessage,
    conn_id: ConnectionId,
    operation_service: &Option<Arc<OperationService>>,
    operation_pipeline: &Option<Arc<tokio::sync::Mutex<OperationPipeline>>>,
    tx: &mpsc::Sender<OutboundMessage>,
) {
    let (Some(classify_svc), Some(pipeline)) = (operation_service, operation_pipeline) else {
        debug!("operation pipeline not configured, dropping message from {:?}", conn_id);
        return;
    };

    // Handle BATCH messages: unpack each inner message and route individually
    if let TopGunMessage::Batch(ref batch_msg) = tg_msg {
        unpack_and_dispatch_batch(batch_msg, conn_id, classify_svc, pipeline, tx).await;
        return;
    }

    // Classify the message into a typed Operation
    match classify_svc.classify(tg_msg, None, CallerOrigin::Client) {
        Ok(mut op) => {
            // Set connection_id so domain services can look up the connection
            op.set_connection_id(conn_id);

            // Route through the full Tower middleware pipeline
            let mut pipeline_guard = pipeline.lock().await;
            match ServiceExt::ready(&mut *pipeline_guard).await {
                Ok(ready_svc) => match ready_svc.call(op).await {
                    Ok(resp) => {
                        drop(pipeline_guard);
                        send_operation_response(resp, tx).await;
                    }
                    Err(e) => {
                        drop(pipeline_guard);
                        debug!("pipeline error for {:?}: {}", conn_id, e);
                    }
                },
                Err(e) => {
                    drop(pipeline_guard);
                    debug!("pipeline not ready for {:?}: {}", conn_id, e);
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

/// Unpacks a BATCH message and dispatches each inner message individually.
///
/// The BATCH `data` field contains length-prefixed binary messages: each
/// inner message is preceded by a 4-byte big-endian u32 length header.
async fn unpack_and_dispatch_batch(
    batch_msg: &topgun_core::messages::BatchMessage,
    conn_id: ConnectionId,
    classify_svc: &OperationService,
    pipeline: &Arc<tokio::sync::Mutex<OperationPipeline>>,
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

        // Classify and route each inner message
        match classify_svc.classify(inner_msg, None, CallerOrigin::Client) {
            Ok(mut op) => {
                op.set_connection_id(conn_id);

                let mut pipeline_guard = pipeline.lock().await;
                match ServiceExt::ready(&mut *pipeline_guard).await {
                    Ok(ready_svc) => match ready_svc.call(op).await {
                        Ok(resp) => {
                            drop(pipeline_guard);
                            send_operation_response(resp, tx).await;
                        }
                        Err(e) => {
                            drop(pipeline_guard);
                            debug!("pipeline error for batch item from {:?}: {}", conn_id, e);
                        }
                    },
                    Err(e) => {
                        drop(pipeline_guard);
                        debug!("pipeline not ready for batch item from {:?}: {}", conn_id, e);
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
/// - `Ack` -> construct OpAck with call_id.to_string() as last_id
/// - `NotImplemented` -> construct Error with code 501
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
                    message: format!("not implemented: {}", service_name),
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
