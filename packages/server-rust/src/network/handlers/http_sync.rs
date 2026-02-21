//! HTTP sync endpoint handler (stub -- full implementation in a later task group).

use axum::extract::State;
use axum::response::IntoResponse;
use bytes::Bytes;

use super::AppState;

/// Handles POST /sync requests with MsgPack-encoded bodies.
///
/// Stub implementation: accepts the body and returns an empty MsgPack response.
/// Full implementation will dispatch through the OperationService.
pub async fn http_sync_handler(
    State(_state): State<AppState>,
    _body: Bytes,
) -> impl IntoResponse {
    (
        [("content-type", "application/msgpack")],
        Vec::<u8>::new(),
    )
}
