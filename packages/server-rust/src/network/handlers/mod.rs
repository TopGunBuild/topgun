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
pub mod auth_validator;
pub mod decode;
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
use dashmap::DashMap;

use super::{ConnectionRegistry, NetworkConfig, ShutdownController};
use crate::cluster::state::ClusterState;
use crate::network::handlers::admin_types::BackfillProgress;
use crate::network::handlers::auth_provider::AuthProvider;
use crate::network::handlers::auth_validator::AuthValidator;
use crate::service::classify::OperationService;
use crate::service::config::ServerConfig;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::domain::counter::CounterRegistry;
use crate::service::domain::index::mutation_observer::IndexObserverFactory;
use crate::service::domain::journal::JournalStore;
use crate::service::domain::messaging::TopicRegistry;
use crate::service::domain::query::QueryRegistry;
use crate::service::domain::search::{
    HybridSearchSubscription, SearchSubscription, SubscriptionRegistry,
};
use crate::service::domain::LockRegistry;
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
    /// Subjects (`Principal.id`) granted the RBAC admin bypass, sourced only from
    /// server-trusted configuration (`TOPGUN_ADMIN_SUBJECTS`). Used to build the
    /// HTTP read-path `PolicyEvaluator` so a `roles:["admin"]` claim cannot grant
    /// the bypass — privilege is anchored to this allow-list. Empty by default.
    pub admin_subjects: Arc<std::collections::HashSet<String>>,
    /// External auth providers for token exchange at POST /api/auth/token.
    /// Empty when token exchange is not configured (endpoint returns 404).
    pub auth_providers: Arc<Vec<Arc<dyn AuthProvider>>>,
    /// Refresh grant store for POST /api/auth/refresh.
    ///
    /// `None` when refresh tokens are disabled (token exchange returns access-only).
    pub refresh_grant_store: Option<Arc<dyn RefreshGrantStore>>,
    /// Custom post-JWT-verification validator.
    /// `None` when no custom validation is configured (default: accept all valid JWTs).
    pub auth_validator: Option<Arc<dyn AuthValidator>>,
    /// Index observer factory for admin index management.
    /// `None` when indexing is not configured.
    pub index_observer_factory: Option<Arc<IndexObserverFactory>>,
    /// Backfill progress tracking for async index creation.
    /// Keyed by `(map_name, attribute)`.
    pub backfill_progress: Arc<DashMap<(String, String), Arc<BackfillProgress>>>,
    /// Lock registry for session-scoped distributed-lock state. `None` in
    /// network-only tests; populated in production wiring so
    /// `handle_socket` can release held locks on WebSocket disconnect.
    pub lock_registry: Option<Arc<LockRegistry>>,
    /// Topic subscription registry. `None` in network-only tests; populated
    /// in production wiring so `handle_socket` can release subscriptions
    /// on WebSocket disconnect.
    pub topic_registry: Option<Arc<TopicRegistry>>,
    /// Counter subscription registry. `None` in network-only tests;
    /// populated in production wiring so `handle_socket` can release
    /// subscriptions on WebSocket disconnect.
    pub counter_registry: Option<Arc<CounterRegistry>>,
    /// Live-query subscription registry. `None` in network-only tests;
    /// populated in production wiring so `handle_socket` can release standing
    /// query subscriptions on WebSocket disconnect.
    pub query_registry: Option<Arc<QueryRegistry>>,
    /// Journal subscription store. `None` in network-only tests; populated in
    /// production wiring so `handle_socket` can release journal subscriptions
    /// on WebSocket disconnect.
    pub journal_store: Option<Arc<JournalStore>>,
    /// Text-search subscription registry. `None` in network-only tests;
    /// populated in production wiring so `handle_socket` can release standing
    /// search subscriptions on WebSocket disconnect.
    pub search_registry: Option<Arc<SubscriptionRegistry<SearchSubscription>>>,
    /// Hybrid-search subscription registry. `None` in network-only tests;
    /// populated in production wiring so `handle_socket` can release standing
    /// hybrid-search subscriptions on WebSocket disconnect.
    pub hybrid_search_registry: Option<Arc<SubscriptionRegistry<HybridSearchSubscription>>>,
    /// Second, independent enforcement layer for the no-auth admin bypass.
    /// When `false`, the `AdminClaims` extractor refuses to synthesize the
    /// local-admin superuser regardless of how the route was mounted, so a
    /// future route-mounting regression cannot re-expose the unauthenticated
    /// admin control plane on a non-loopback bind.
    pub admin_enabled: bool,
    /// Filesystem path of the vector-index descriptor sidecar JSON, resolved
    /// from `TOPGUN_VECTOR_INDEX_PATH` once at the construction boundary so the
    /// admin handlers never read process-global env directly. `None` falls back
    /// to the default `./vector_indexes.json`; tests inject a temp path here
    /// instead of mutating the process environment (test-isolation seam).
    pub vector_index_path: Option<std::path::PathBuf>,
}

impl AppState {
    /// Constructor for test contexts.
    ///
    /// Returns an `AppState` with all `Option<_>` fields set to `None`
    /// and required fields set to in-memory defaults. Tests override
    /// specific fields with struct-update syntax:
    ///
    /// ```ignore
    /// let state = AppState {
    ///     operation_service: Some(svc),
    ///     ..AppState::for_test()
    /// };
    /// ```
    ///
    /// Production code paths (`module.rs`, `topgun_server.rs`,
    /// `load_harness/main.rs`) continue to construct `AppState`
    /// explicitly so field coverage is compiler-checked.
    #[must_use]
    pub fn for_test() -> Self {
        Self {
            registry: Arc::new(ConnectionRegistry::new()),
            // Fresh ShutdownController with an unbroadcast signal so tests
            // never observe a shutdown-in-progress state unless they drive it.
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
            admin_subjects: Arc::new(std::collections::HashSet::new()),
            // Empty slice because tests that need auth providers supply them
            // explicitly via struct-update syntax.
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: None,
            index_observer_factory: None,
            // Fresh empty map; tests that drive backfill supply their own Arc.
            backfill_progress: Arc::new(DashMap::new()),
            lock_registry: None,
            topic_registry: None,
            counter_registry: None,
            query_registry: None,
            journal_store: None,
            search_registry: None,
            hybrid_search_registry: None,
            // Default-safe: tests exercise the admin plane as enabled unless they
            // explicitly override this to assert the disabled-plane guard.
            admin_enabled: true,
            // None → default path. Tests that persist descriptors override this
            // with a temp path instead of mutating TOPGUN_VECTOR_INDEX_PATH.
            vector_index_path: None,
        }
    }
}
