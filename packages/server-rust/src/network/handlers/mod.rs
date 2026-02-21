//! HTTP and WebSocket handler definitions for the TopGun server.
//!
//! This module defines `AppState` (the shared state carried through axum
//! extractors) and re-exports all handler functions for convenient access
//! when building the router.

pub mod health;
pub mod http_sync;
pub mod websocket;

pub use health::{health_handler, liveness_handler, readiness_handler};
pub use http_sync::http_sync_handler;
pub use websocket::ws_upgrade_handler;

use std::sync::Arc;
use std::time::Instant;

use super::{ConnectionRegistry, NetworkConfig, ShutdownController};

/// Shared application state passed to all axum handlers via `State` extraction.
///
/// Holds `Arc` references to shared resources so cloning is cheap.
/// This struct will grow as future modules add fields (e.g., OperationService,
/// ClusterState).
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
}
