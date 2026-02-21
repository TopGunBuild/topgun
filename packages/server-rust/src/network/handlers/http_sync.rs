//! HTTP sync endpoint handler for `MsgPack`-encoded request/response.
//!
//! Provides a POST /sync endpoint that accepts `MsgPack` bodies and returns
//! `MsgPack` responses. This is the HTTP fallback transport for clients that
//! cannot maintain a WebSocket connection (e.g., behind restrictive proxies).

use axum::extract::State;
use axum::response::IntoResponse;
use bytes::Bytes;

use super::AppState;

/// Content-Type header value for `MsgPack` responses.
const MSGPACK_CONTENT_TYPE: &str = "application/msgpack";

/// Handles POST /sync requests with `MsgPack`-encoded bodies.
///
/// Stub implementation: accepts the body and returns an empty `MsgPack` array.
/// Full implementation will decode the request body into operations, dispatch
/// them through the `OperationService`, and return the aggregated results.
///
/// # Panics
///
/// Panics if serializing an empty `Vec<()>` fails, which is infallible in
/// practice.
pub async fn http_sync_handler(
    State(_state): State<AppState>,
    _body: Bytes,
) -> impl IntoResponse {
    // Encode an empty array as the stub response. Using to_vec_named
    // ensures field names are preserved in the MsgPack output, matching
    // the wire format that clients expect.
    let empty_response =
        rmp_serde::to_vec_named(&Vec::<()>::new()).expect("empty vec serialization cannot fail");

    ([("content-type", MSGPACK_CONTENT_TYPE)], empty_response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{ConnectionRegistry, NetworkConfig, ShutdownController};
    use std::sync::Arc;
    use std::time::Instant;

    fn test_state() -> AppState {
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
        }
    }

    #[tokio::test]
    async fn http_sync_handler_returns_msgpack_content_type() {
        let state = test_state();
        let body = Bytes::from_static(b"");

        let response = http_sync_handler(State(state), body).await.into_response();
        let content_type = response
            .headers()
            .get("content-type")
            .expect("content-type header must be present");
        assert_eq!(content_type, "application/msgpack");
    }

    #[tokio::test]
    async fn http_sync_handler_returns_valid_msgpack_body() {
        let state = test_state();
        let body = Bytes::from_static(b"\x90"); // empty MsgPack array

        let response = http_sync_handler(State(state), body).await.into_response();
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .expect("body read must succeed");

        // Verify the response body is a valid MsgPack-encoded empty array.
        let decoded: Vec<()> =
            rmp_serde::from_slice(&body).expect("response must be valid MsgPack");
        assert!(decoded.is_empty());
    }

    #[tokio::test]
    async fn http_sync_handler_returns_200() {
        let state = test_state();
        let body = Bytes::from_static(b"");

        let response = http_sync_handler(State(state), body).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }
}
