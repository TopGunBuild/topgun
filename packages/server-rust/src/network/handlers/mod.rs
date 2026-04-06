//! HTTP and WebSocket handler definitions for the `TopGun` server.
//!
//! This module defines `AppState` (the shared state carried through axum
//! extractors) and re-exports all handler functions for convenient access
//! when building the router.

pub mod admin;
pub mod admin_auth;
pub mod admin_types;
pub mod auth;
pub mod auth_provider;
pub mod health;
pub mod http_sync;
pub mod metrics_endpoint;
pub mod refresh;
pub mod refresh_types;
pub mod token_exchange;
pub mod websocket;

pub use health::{health_handler, liveness_handler, readiness_handler};
pub use http_sync::http_sync_handler;
pub use metrics_endpoint::metrics_handler;
pub use refresh::refresh_handler;
pub use refresh_types::{RefreshGrant, RefreshGrantStore};
pub use token_exchange::token_exchange_handler;
pub use websocket::ws_upgrade_handler;

use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;

use super::{ConnectionRegistry, NetworkConfig, ShutdownController};
use crate::cluster::state::ClusterState;
use crate::network::handlers::auth_provider::AuthProvider;
use crate::service::classify::OperationService;
use crate::service::config::ServerConfig;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::middleware::ObservabilityHandle;
use crate::service::policy::PolicyStore;
use crate::storage::factory::RecordStoreFactory;

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
    /// Partition-based operation dispatcher that routes operations to
    /// per-worker pipelines via MPSC channels.
    ///
    /// `None` in network-only tests that do not wire the service layer.
    pub dispatcher: Option<Arc<PartitionDispatcher>>,
    /// JWT secret for verifying authentication tokens.
    ///
    /// `None` when auth is not configured (existing network tests).
    pub jwt_secret: Option<String>,
    /// Cluster state for admin cluster status endpoint.
    ///
    /// `None` in single-node mode or when cluster is not configured.
    pub cluster_state: Option<Arc<ClusterState>>,
    /// Record store factory for admin map enumeration.
    ///
    /// `None` in network-only tests that do not wire the storage layer.
    pub store_factory: Option<Arc<RecordStoreFactory>>,
    /// Hot-reloadable server configuration wrapped in `ArcSwap`.
    ///
    /// Introduced specifically for admin settings hot-reload. Existing services
    /// continue using their own `Arc<ServerConfig>` references unchanged.
    /// `None` when admin endpoints are not configured.
    pub server_config: Option<Arc<ArcSwap<ServerConfig>>>,
    /// Policy store for permission policy CRUD and evaluation.
    /// `None` when policy engine is not configured.
    pub policy_store: Option<Arc<dyn PolicyStore>>,
    /// External auth providers for token exchange at POST /api/auth/token.
    /// Empty when token exchange is not configured (endpoint returns 404).
    pub auth_providers: Arc<Vec<Arc<dyn AuthProvider>>>,
    /// Refresh grant store for POST /api/auth/refresh.
    ///
    /// `None` when refresh tokens are disabled (token exchange returns access-only).
    pub refresh_grant_store: Option<Arc<dyn RefreshGrantStore>>,
}
