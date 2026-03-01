//! Prometheus metrics endpoint handler.
//!
//! Serves the current metric state in Prometheus text exposition format (v0.0.4)
//! at `GET /metrics`.  The handler reads the active connection count from the
//! registry on each scrape (pull model) and then delegates to
//! `ObservabilityHandle::render_metrics` for the full text output.
//!
//! When `AppState::observability` is `None` (e.g., in unit tests that do not call
//! `init_observability`) the handler returns an empty-body 200 response, ensuring
//! that existing tests are not broken.

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use super::AppState;

/// Content-Type required by the Prometheus text exposition format v0.0.4.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// `GET /metrics` handler — returns Prometheus text exposition format.
///
/// Records the current active connection count as a gauge on each scrape,
/// then renders all metrics to the Prometheus text format and returns them
/// with the required `Content-Type` header.
///
/// Returns HTTP 200 with an empty body when observability is not initialised.
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Update the active-connections gauge on every scrape (pull model).
    // This avoids needing to hook into every connect/disconnect event.
    metrics::gauge!("topgun_active_connections").set(state.registry.count() as f64);

    match &state.observability {
        Some(handle) => {
            let body = handle.render_metrics();
            Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
                )
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        None => {
            // Graceful degradation: return 200 with empty body when observability
            // is not configured (e.g., in test environments).
            Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
                )
                .body(axum::body::Body::empty())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{ConnectionRegistry, NetworkConfig, ShutdownController};
    use std::sync::Arc;
    use std::time::Instant;

    fn test_state_no_obs() -> AppState {
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
        }
    }

    #[tokio::test]
    async fn metrics_handler_returns_200_without_observability() {
        let state = test_state_no_obs();
        let resp = metrics_handler(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn metrics_handler_content_type_without_observability() {
        let state = test_state_no_obs();
        let resp = metrics_handler(State(state)).await.into_response();
        let ct = resp.headers().get("content-type").expect("content-type present");
        assert_eq!(ct, PROMETHEUS_CONTENT_TYPE);
    }

    #[tokio::test]
    async fn metrics_handler_returns_200_with_observability() {
        use crate::service::middleware::init_observability;
        let obs = init_observability();
        let mut state = test_state_no_obs();
        state.observability = Some(Arc::new(obs));

        let resp = metrics_handler(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").expect("content-type present");
        assert_eq!(ct, PROMETHEUS_CONTENT_TYPE);
    }
}
