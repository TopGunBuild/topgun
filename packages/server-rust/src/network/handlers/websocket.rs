//! WebSocket upgrade handler with inbound/outbound message loops.
//!
//! Uses the socket-split pattern: the WebSocket is split into a sender
//! (owned by the outbound task) and a receiver (owned by the inbound loop).
//! This avoids holding a single mutable reference across concurrent reads
//! and writes.

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::State;
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};

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
/// The function splits the socket into sender/receiver halves, spawns an
/// outbound task for writes, and runs the inbound loop in the current task.
/// When the inbound loop exits (client disconnect or error), the connection
/// is removed from the registry.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (handle, rx) = state
        .registry
        .register(ConnectionKind::Client, &state.config.connection);
    let conn_id = handle.id;

    debug!("WebSocket connected: {:?}", conn_id);

    let (sender, mut receiver) = socket.split();

    // Outbound task owns the write half of the socket and drains
    // the mpsc channel, coalescing messages when multiple are ready.
    let outbound_handle = tokio::spawn(outbound_task(sender, rx));

    // Inbound loop: read messages from the client until disconnect or error.
    while let Some(result) = receiver.next().await {
        match result {
            Ok(Message::Binary(data)) => {
                debug!(
                    "received {} bytes from connection {:?}",
                    data.len(),
                    conn_id
                );
                // Stub: no OperationService dispatch yet. Binary messages
                // are logged but not processed.
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
