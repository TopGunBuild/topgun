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

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::State;
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use tokio::sync::mpsc;
use topgun_core::messages::{AuthAckData, Message as TopGunMessage};
use tracing::{debug, warn};

use super::auth::AuthHandler;
use super::AppState;
use crate::network::{ConnectionKind, OutboundMessage};

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
                    // Authenticated: dispatch through operation pipeline (G3-S2)
                    debug!(
                        "received {} bytes from authenticated connection {:?}",
                        data.len(),
                        conn_id
                    );
                    // Pipeline dispatch will be implemented in G3-S2
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
