//! HTTP and WebSocket handler definitions for the `TopGun` server.
//!
//! This module defines `AppState` (the shared state carried through axum
//! extractors) and re-exports all handler functions for convenient access
//! when building the router.

pub mod auth;
pub mod health;
pub mod http_sync;
pub mod metrics_endpoint;
pub mod websocket;

pub use health::{health_handler, liveness_handler, readiness_handler};
pub use http_sync::http_sync_handler;
pub use metrics_endpoint::metrics_handler;
pub use websocket::ws_upgrade_handler;

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

use super::{ConnectionRegistry, NetworkConfig, ShutdownController};
use crate::service::classify::OperationService;
use crate::service::middleware::ObservabilityHandle;
use crate::service::operation::OperationPipeline;

/// Shared application state passed to all axum handlers via `State` extraction.
///
/// Holds `Arc` references to shared resources so cloning is cheap.
/// This struct will grow as future modules add fields (e.g., `OperationService`,
/// `ClusterState`).
#[derive(Clone)]
pub struct AppState {
    /// Registry of all active WebSocket and cluster peer connections.
    pub registry: Arc<ConnectionRegistry>,
    /// Graceful shutdown controller with health state and in-flight tracking.
    pub shutdown: Arc<ShutdownController>,
    /// Network configuration (bind address, TLS, per-connection settings).
    pub config: Arc<NetworkConfig>,
    /// Server process start time, used for uptime calculation.
    pub start_time: Instant,
    /// Prometheus metrics handle for the `/metrics` endpoint.
    ///
    /// `None` in test environments that do not call `init_observability()`,
    /// ensuring existing tests compile without modification.
    pub observability: Option<Arc<ObservabilityHandle>>,
    /// Operation classifier that converts `Message` into typed `Operation` values.
    ///
    /// `None` in network-only tests that do not wire the service layer.
    pub operation_service: Option<Arc<OperationService>>,
    /// Full Tower middleware pipeline (`LoadShed` -> `Timeout` -> `Metrics` -> `Router`).
    /// Wrapped in `Mutex` because `Service::call()` requires `&mut self`.
    ///
    /// `None` in network-only tests that do not wire the service layer.
    pub operation_pipeline: Option<Arc<Mutex<OperationPipeline>>>,
    /// JWT secret for verifying authentication tokens.
    ///
    /// `None` when auth is not configured (existing network tests).
    pub jwt_secret: Option<String>,
}
