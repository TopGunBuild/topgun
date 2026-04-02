//! HTTP sync endpoint handler for `MsgPack`-encoded request/response.
//!
//! Provides a POST /sync endpoint that accepts `MsgPack` bodies and returns
//! `MsgPack` responses. This is the HTTP fallback transport for clients that
//! cannot maintain a WebSocket connection (e.g., behind restrictive proxies).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bytes::Bytes;
use topgun_core::hash_to_partition;
use topgun_core::messages::{HttpSyncAck, HttpSyncError, HttpSyncRequest, HttpSyncResponse};
use topgun_core::Timestamp;

use super::AppState;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::operation::CallerOrigin;

/// Content-Type header value for `MsgPack` responses.
const MSGPACK_CONTENT_TYPE: &str = "application/msgpack";

/// Returns the current wall-clock time as a `Timestamp` when the HLC is unavailable.
fn wall_clock_timestamp() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Timestamp {
        millis,
        counter: 0,
        node_id: String::new(),
    }
}

/// Handles POST /sync requests with `MsgPack`-encoded bodies.
///
/// Decodes the request body as `HttpSyncRequest`, dispatches operations through
/// the `OperationService`/`PartitionDispatcher` pipeline, and returns an
/// `HttpSyncResponse` with the server's current HLC timestamp and an ack for
/// any submitted operations.
///
/// Returns HTTP 400 with a JSON error body when the request body is not valid
/// MsgPack or cannot be decoded as `HttpSyncRequest`.
///
/// When `operation_service` or `dispatcher` is absent (test environments without
/// service wiring), returns a minimal response with only `server_hlc` populated.
pub async fn http_sync_handler(
    State(state): State<AppState>,
    body: Bytes,
) -> impl IntoResponse {
    // Obtain the server's current HLC timestamp. Fall back to wall-clock time
    // if no operation_service is wired (e.g., network-only test environments).
    let server_hlc: Timestamp = state
        .operation_service
        .as_ref()
        .map(|s| s.now())
        .unwrap_or_else(wall_clock_timestamp);

    // Decode the request body. An empty body is treated as an empty request
    // only when the body length is zero; any non-zero body must be valid MsgPack.
    let request: HttpSyncRequest = if body.is_empty() {
        HttpSyncRequest::default()
    } else {
        match rmp_serde::from_slice::<HttpSyncRequest>(&body) {
            Ok(req) => req,
            Err(e) => {
                let json = format!(r#"{{"code":400,"message":"invalid MsgPack body: {e}"}}"#);
                return (
                    StatusCode::BAD_REQUEST,
                    [("content-type", "application/json")],
                    json.into_bytes(),
                )
                    .into_response();
            }
        }
    };

    // When neither service is wired, return a minimal response with only server_hlc.
    let (Some(classify_svc), Some(dispatcher)) =
        (state.operation_service.as_ref(), state.dispatcher.as_ref())
    else {
        let response = HttpSyncResponse {
            server_hlc,
            ..Default::default()
        };
        return msgpack_response(response);
    };

    let mut http_response = HttpSyncResponse {
        server_hlc,
        ..Default::default()
    };

    // Dispatch operations if present and non-empty.
    if let Some(ops) = request.operations {
        if !ops.is_empty() {
            let last_id = ops
                .last()
                .and_then(|op| op.id.clone())
                .unwrap_or_else(|| "unknown".to_string());

            // Group ops by partition so each group targets one partition worker.
            let mut partition_groups: HashMap<u32, Vec<topgun_core::messages::ClientOp>> =
                HashMap::new();
            for op in ops {
                let partition_id = hash_to_partition(&op.key);
                partition_groups.entry(partition_id).or_default().push(op);
            }

            // Build sub-batch operations up front, then dispatch concurrently.
            let mut sub_ops: Vec<crate::service::operation::Operation> =
                Vec::with_capacity(partition_groups.len());
            for (partition_id, group_ops) in partition_groups {
                let op = classify_svc.classify_op_batch_for_partition(
                    group_ops,
                    partition_id,
                    None,
                    CallerOrigin::System,
                    None,
                    None,
                );
                sub_ops.push(op);
            }

            // Dispatch all sub-batches concurrently.
            let mut join_set = tokio::task::JoinSet::new();
            for sub_op in sub_ops {
                let dispatcher: Arc<PartitionDispatcher> = Arc::clone(dispatcher);
                join_set.spawn(async move { dispatcher.dispatch(sub_op).await });
            }

            // Collect results; record any dispatch errors.
            let mut dispatch_error: Option<String> = None;
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok(_resp)) => {}
                    Ok(Err(e)) => {
                        dispatch_error = Some(format!("{e}"));
                    }
                    Err(join_err) => {
                        dispatch_error = Some(format!("join error: {join_err}"));
                    }
                }
            }

            if let Some(msg) = dispatch_error {
                let errors = http_response.errors.get_or_insert_with(Vec::new);
                errors.push(HttpSyncError {
                    code: 500,
                    message: msg,
                    context: None,
                });
            } else {
                http_response.ack = Some(HttpSyncAck {
                    last_id,
                    results: None,
                });
            }
        }
    }

    msgpack_response(http_response)
}

/// Serializes `response` as MsgPack and wraps it in an HTTP 200 response.
fn msgpack_response(response: HttpSyncResponse) -> axum::response::Response {
    match rmp_serde::to_vec_named(&response) {
        Ok(bytes) => (
            StatusCode::OK,
            [("content-type", MSGPACK_CONTENT_TYPE)],
            bytes,
        )
            .into_response(),
        Err(e) => {
            let json = format!(r#"{{"code":500,"message":"serialization error: {e}"}}"#);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                json.into_bytes(),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{ConnectionRegistry, NetworkConfig, ShutdownController};
    use std::sync::Arc;
    use std::time::Instant;
    use topgun_core::messages::HttpSyncRequest;
    use topgun_core::Timestamp;

    fn test_state() -> AppState {
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: None,
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
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
    async fn http_sync_handler_returns_200_for_empty_body() {
        let state = test_state();
        let body = Bytes::from_static(b"");

        let response = http_sync_handler(State(state), body).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn http_sync_handler_returns_valid_server_hlc_for_empty_request() {
        let state = test_state();

        // Encode a minimal HttpSyncRequest (no operations) as MsgPack.
        let req = HttpSyncRequest {
            client_id: "test-client".to_string(),
            client_hlc: Timestamp {
                millis: 1_000,
                counter: 0,
                node_id: "node-1".to_string(),
            },
            ..Default::default()
        };
        let body = Bytes::from(rmp_serde::to_vec_named(&req).unwrap());

        let response = http_sync_handler(State(state), body).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body read must succeed");
        let decoded: HttpSyncResponse =
            rmp_serde::from_slice(&body_bytes).expect("response must be valid MsgPack");

        // server_hlc.millis must be a positive wall-clock timestamp.
        assert!(decoded.server_hlc.millis > 0, "server_hlc.millis must be > 0");
    }

    #[tokio::test]
    async fn http_sync_handler_returns_400_for_malformed_body() {
        let state = test_state();
        // Two bytes that are not a valid MsgPack-encoded HttpSyncRequest.
        let body = Bytes::from_static(&[0xFF, 0xFF]);

        let response = http_sync_handler(State(state), body).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}
