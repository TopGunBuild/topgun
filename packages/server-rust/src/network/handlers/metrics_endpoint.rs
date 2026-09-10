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
    // The cast from usize to f64 may lose precision for counts > 2^53, which
    // is not a concern for active connection counts.
    #[allow(clippy::cast_precision_loss)]
    let connection_count = state.registry.count() as f64;
    metrics::gauge!("topgun_active_connections").set(connection_count);

    match &state.observability {
        Some(handle) => {
            // Pull the conjunct snapshot fresh on every scrape — this is the only call
            // site, so the render below always reflects the state at this instant rather
            // than a value cached from an earlier request or a background tick.
            if let Some(frontier) = &state.frontier {
                frontier.publish_conjunct_snapshot();
            }
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
    use std::sync::Arc;

    use super::*;

    fn test_state_no_obs() -> AppState {
        AppState::for_test()
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
        let ct = resp
            .headers()
            .get("content-type")
            .expect("content-type present");
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
        let ct = resp
            .headers()
            .get("content-type")
            .expect("content-type present");
        assert_eq!(ct, PROMETHEUS_CONTENT_TYPE);
    }

    /// A scrape with a frontier attached must publish a conjunct snapshot before
    /// rendering, so the body it returns carries every one of the 21 conjunct series —
    /// not just the ones a prior test happened to touch.
    ///
    /// Only presence is asserted: the process-global recorder is shared across the
    /// test binary, so a concurrently running test can move a value between the two
    /// scrapes this test would otherwise compare.
    #[tokio::test]
    async fn metrics_handler_publishes_conjunct_snapshot_before_render() {
        use crate::service::middleware::init_observability;
        use crate::tombstone_frontier::{
            METRIC_PRUNE_CONJUNCT_CEILING, METRIC_PRUNE_CONJUNCT_CLAIMS,
            METRIC_PRUNE_CONJUNCT_CLAIM_LAG_MAX, METRIC_PRUNE_CONJUNCT_CLAIM_LAG_P50,
            METRIC_PRUNE_CONJUNCT_CLAIM_LAG_P99, METRIC_PRUNE_CONJUNCT_CURRENT_EPOCH,
            METRIC_PRUNE_CONJUNCT_DURABLE_WATERMARK, METRIC_PRUNE_CONJUNCT_DURABLE_WATERMARK_LAG,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_BOTH,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_CLAIM_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_DURABILITY_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_NEITHER,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_UNSLOTTED,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_BOTH, METRIC_PRUNE_CONJUNCT_RETAINED_REFS_CLAIM_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_DURABILITY_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_NEITHER,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_OPEN_EPOCH,
            METRIC_PRUNE_CONJUNCT_RETAINED_STAMPED_BYTES,
            METRIC_PRUNE_CONJUNCT_RETAINED_STAMPED_BYTES_OPEN_EPOCH,
            METRIC_PRUNE_CONJUNCT_SNAPSHOTS_TOTAL,
        };
        use crate::tombstone_frontier_impl::TombstoneFrontier;

        // `init_observability()` first, so the recorder is bound before anything else
        // touches a metric; a series touched before binding would be lost to this render.
        let obs = init_observability();
        let mut state = test_state_no_obs();
        state.observability = Some(Arc::new(obs));
        state.frontier = Some(Arc::new(TombstoneFrontier::new(None)));

        let resp = metrics_handler(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("readable body");
        let body = String::from_utf8(body_bytes.to_vec()).expect("utf-8 body");

        for name in [
            METRIC_PRUNE_CONJUNCT_SNAPSHOTS_TOTAL,
            METRIC_PRUNE_CONJUNCT_CURRENT_EPOCH,
            METRIC_PRUNE_CONJUNCT_CEILING,
            METRIC_PRUNE_CONJUNCT_DURABLE_WATERMARK,
            METRIC_PRUNE_CONJUNCT_DURABLE_WATERMARK_LAG,
            METRIC_PRUNE_CONJUNCT_CLAIMS,
            METRIC_PRUNE_CONJUNCT_CLAIM_LAG_P50,
            METRIC_PRUNE_CONJUNCT_CLAIM_LAG_P99,
            METRIC_PRUNE_CONJUNCT_CLAIM_LAG_MAX,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_CLAIM_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_DURABILITY_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_BOTH,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_NEITHER,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_CLAIM_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_DURABILITY_ONLY,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_BOTH,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_NEITHER,
            METRIC_PRUNE_CONJUNCT_RETAINED_STAMPED_BYTES,
            METRIC_PRUNE_CONJUNCT_RETAINED_EPOCHS_UNSLOTTED,
            METRIC_PRUNE_CONJUNCT_RETAINED_REFS_OPEN_EPOCH,
            METRIC_PRUNE_CONJUNCT_RETAINED_STAMPED_BYTES_OPEN_EPOCH,
        ] {
            assert!(
                body.contains(name),
                "conjunct series {name} missing from a scrape with a frontier attached; \
                 body was:\n{body}"
            );
        }
    }
}
