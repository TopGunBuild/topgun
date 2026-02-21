//! WebSocket upgrade handler (stub -- full implementation in a later task group).

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::response::IntoResponse;

use super::AppState;

/// Upgrades an HTTP connection to a WebSocket connection.
///
/// Stub implementation: accepts the upgrade but does not process messages.
/// Full implementation adds inbound/outbound message loops with coalescing.
pub async fn ws_upgrade_handler(
    State(_state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|_socket| async {})
}
