//! Health, liveness, and readiness endpoint handlers.
//!
//! These handlers expose server health information for orchestrators
//! (Kubernetes, load balancers) and operational monitoring.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;

use super::AppState;
use crate::network::HealthState;

/// Returns detailed health information as JSON.
///
/// Always returns 200 -- the `state` field in the response body indicates
/// whether the server is actually healthy. This lets monitoring tools
/// distinguish between "server is up but draining" vs "server is down".
pub async fn health_handler(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let health = state.shutdown.health_state();
    let connections = state.registry.count();
    let in_flight = state.shutdown.in_flight_count();
    let uptime_secs = state.start_time.elapsed().as_secs();

    Json(json!({
        "state": health.as_str(),
        "connections": connections,
        "in_flight": in_flight,
        "uptime_secs": uptime_secs,
    }))
}

/// Kubernetes liveness probe -- always returns 200 OK.
///
/// The liveness probe only checks whether the process is running and
/// responsive. It intentionally does not check downstream dependencies
/// or health state, because a failed liveness probe triggers a pod restart.
pub async fn liveness_handler() -> StatusCode {
    StatusCode::OK
}

/// Kubernetes readiness probe -- returns 200 when ready, 503 otherwise.
///
/// Returns 503 during startup (before `set_ready()` is called), during
/// graceful shutdown (Draining state), and after stop. This removes the
/// pod from the Service's endpoint list so no new traffic is routed to it.
pub async fn readiness_handler(State(state): State<AppState>) -> StatusCode {
    if state.shutdown.health_state() == HealthState::Ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
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
    async fn health_handler_returns_json_with_all_fields() {
        let state = test_state();
        state.shutdown.set_ready();

        let response = health_handler(State(state)).await;
        let json = response.0;

        assert_eq!(json["state"], "ready");
        assert_eq!(json["connections"], 0);
        assert_eq!(json["in_flight"], 0);
        assert!(json["uptime_secs"].is_number());
    }

    #[tokio::test]
    async fn health_handler_reports_starting_state() {
        let state = test_state();
        let response = health_handler(State(state)).await;
        assert_eq!(response.0["state"], "starting");
    }

    #[tokio::test]
    async fn health_handler_reports_draining_state() {
        let state = test_state();
        state.shutdown.set_ready();
        state.shutdown.trigger_shutdown();

        let response = health_handler(State(state)).await;
        assert_eq!(response.0["state"], "draining");
    }

    #[tokio::test]
    async fn health_handler_reports_connection_count() {
        let state = test_state();
        let config = crate::network::ConnectionConfig::default();
        let (_handle, _rx) = state.registry.register(
            crate::network::ConnectionKind::Client,
            &config,
        );

        let response = health_handler(State(state)).await;
        assert_eq!(response.0["connections"], 1);
    }

    #[tokio::test]
    async fn health_handler_reports_in_flight_count() {
        let state = test_state();
        let _guard = state.shutdown.in_flight_guard();

        let response = health_handler(State(state)).await;
        assert_eq!(response.0["in_flight"], 1);
    }

    #[tokio::test]
    async fn liveness_handler_always_returns_200() {
        let status = liveness_handler().await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_handler_returns_200_when_ready() {
        let state = test_state();
        state.shutdown.set_ready();

        let status = readiness_handler(State(state)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_handler_returns_503_when_starting() {
        let state = test_state();
        let status = readiness_handler(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readiness_handler_returns_503_when_draining() {
        let state = test_state();
        state.shutdown.set_ready();
        state.shutdown.trigger_shutdown();

        let status = readiness_handler(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
