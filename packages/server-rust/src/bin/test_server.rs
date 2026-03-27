//! Test server binary for integration tests.
//!
//! Wires all 7 domain services, starts on port 0, and prints `PORT=<number>`
//! to stdout. The TS test harness reads the port from stdout to connect.
//!
//! Uses `NullDataStore` (no `PostgreSQL` dependency) and JWT secret `test-e2e-secret`
//! to match the TS test helpers.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use axum::routing::get;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tokio::signal;
use topgun_core::{SystemClock, HLC};

use topgun_server::cluster::state::ClusterState;
use topgun_server::cluster::types::ClusterConfig;
use topgun_server::network::config::NetworkConfig;
use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::network::handlers::AppState;
use topgun_server::network::shutdown::ShutdownController;
use topgun_server::service::config::ServerConfig;
use topgun_server::service::domain::coordination::CoordinationService;
use topgun_server::service::domain::crdt::CrdtService;
use topgun_server::service::domain::messaging::MessagingService;
use topgun_server::service::domain::schema::SchemaService;
use topgun_server::service::domain::persistence::PersistenceService;
use topgun_server::service::domain::query::{QueryRegistry, QueryService};
use topgun_server::service::domain::search::{
    SearchConfig, SearchMutationObserver, SearchRegistry, SearchService, TantivyMapIndex,
};
use topgun_server::service::domain::sync::SyncService;
use topgun_server::service::dispatch::{DispatchConfig, PartitionDispatcher};
use topgun_server::service::middleware::build_operation_pipeline;
use topgun_server::service::operation::service_names;
use topgun_server::service::router::OperationRouter;
use topgun_server::service::security::{SecurityConfig, WriteValidator};
use topgun_server::service::OperationService;
use topgun_server::storage::datastores::NullDataStore;
use topgun_server::storage::factory::{ObserverFactory, RecordStoreFactory};
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use topgun_server::storage::mutation_observer::MutationObserver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let (classify_svc, dispatcher, connection_registry) = build_services();

    // Build the AppState with all services wired
    let shutdown = Arc::new(ShutdownController::new());
    let state = AppState {
        registry: connection_registry,
        shutdown: Arc::clone(&shutdown),
        config: Arc::new(NetworkConfig::default()),
        start_time: Instant::now(),
        observability: None,
        operation_service: Some(classify_svc),
        dispatcher: Some(Arc::new(dispatcher)),
        jwt_secret: Some("test-e2e-secret".to_string()),
        cluster_state: None,
        store_factory: None,
        server_config: None,
    };

    // Build the axum router with state.
    // Serve WebSocket on both /ws (integration tests) and / (browser clients).
    // Include /health so Docker Compose healthchecks and inter-container probes succeed.
    let ws_handler = get(topgun_server::network::handlers::ws_upgrade_handler);
    let health_handler = get(topgun_server::network::handlers::health_handler);
    let app = axum::Router::new()
        .route("/ws", ws_handler.clone())
        .route("/", ws_handler)
        .route("/health", health_handler)
        .with_state(state);

    // Use PORT env var if set (for manual testing), otherwise OS-assigned
    let bind_port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // Bind to all interfaces so inter-container traffic (Docker networking) reaches
    // the server. TOPGUN_BIND_ADDR overrides the default for environments that
    // require loopback-only binding.
    let bind_addr = std::env::var("TOPGUN_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let listener = TcpListener::bind(format!("{bind_addr}:{bind_port}")).await?;
    let port = listener.local_addr()?.port();

    // Print port to stdout so the TS test harness can read it
    println!("PORT={port}");

    // Mark the server as ready
    shutdown.set_ready();

    // Serve until SIGTERM or SIGINT
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Observer factory that creates a `SearchMutationObserver` for every map.
///
/// Shares the same `indexes`, `search_registry`, and `needs_population` with
/// `SearchService` so that writes indexed by the observer are immediately visible
/// to search queries and live subscriptions.
struct SearchObserverFactory {
    search_registry: Arc<SearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
    /// Shared with `SearchService` so the observer can signal that an index needs
    /// population when writes were skipped due to no active subscriptions.
    needs_population: Arc<DashMap<String, AtomicBool>>,
}

impl ObserverFactory for SearchObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        _partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        // Use fast batch parameters for integration tests to keep test latency low.
        // Production uses SearchConfig::default() (100ms / 500 threshold).
        let config = SearchConfig {
            batch_interval_ms: 16,
            batch_flush_threshold: 100,
        };
        let observer = SearchMutationObserver::new(
            map_name.to_string(),
            Arc::clone(&self.search_registry),
            Arc::clone(&self.indexes),
            Arc::clone(&self.connection_registry),
            config,
            Arc::clone(&self.needs_population),
        );
        Some(Arc::new(observer))
    }
}


/// Wires all 7 domain services and builds the partition dispatcher.
///
/// Follows the `setup()` pattern from `packages/server-rust/src/lib.rs:63-148`.
/// Domain services are `Arc`-wrapped and shared across all worker pipelines.
/// Each worker gets its own `OperationRouter` + `OperationPipeline` via a
/// factory closure passed to `PartitionDispatcher::new()`.
#[allow(clippy::too_many_lines)]
fn build_services() -> (
    Arc<OperationService>,
    PartitionDispatcher,
    Arc<ConnectionRegistry>,
) {
    let config = ServerConfig {
        node_id: "test-server-node".to_string(),
        default_operation_timeout_ms: 5000,
        max_concurrent_operations: 100,
        gc_interval_ms: 60_000,
        ..ServerConfig::default()
    };

    let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
        config.node_id.clone(),
        Box::new(SystemClock),
    )));

    let classify_svc = Arc::new(OperationService::new(
        Arc::clone(&hlc),
        Arc::new(config.clone()),
    ));

    let cluster_config = Arc::new(ClusterConfig::default());
    let (cluster_state, _rx) =
        ClusterState::new(cluster_config, "test-server-node".to_string());
    let cluster_state = Arc::new(cluster_state);
    let connection_registry = Arc::new(ConnectionRegistry::new());

    // Shared search state: indexes, registry, and needs_population are shared
    // between SearchObserverFactory (write path) and SearchService (query path).
    // needs_population signals when writes were skipped due to no subscriptions,
    // so SearchService can lazily repopulate the index on first search query.
    let search_registry = Arc::new(SearchRegistry::new());
    let search_indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let search_needs_population: Arc<DashMap<String, AtomicBool>> =
        Arc::new(DashMap::new());

    let search_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(SearchObserverFactory {
            search_registry: Arc::clone(&search_registry),
            indexes: Arc::clone(&search_indexes),
            connection_registry: Arc::clone(&connection_registry),
            needs_population: Arc::clone(&search_needs_population),
        });

    // QueryRegistry shared between CrdtService (broadcast_query_updates) and QueryService.
    // QueryMutationObserver is no longer in the observer chain -- CrdtService handles
    // QUERY_UPDATE broadcast directly with writer exclusion and field projection.
    let query_registry = Arc::new(QueryRegistry::new());

    // MerkleSyncManager must be created before RecordStoreFactory so the
    // MerkleObserverFactory can be included in with_observer_factories().
    let merkle_manager = Arc::new(MerkleSyncManager::default());

    let merkle_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(MerkleObserverFactory::new(Arc::clone(&merkle_manager)));

    #[allow(unused_mut)]
    let mut observer_factories: Vec<Arc<dyn ObserverFactory>> = vec![
        search_observer_factory,
        merkle_observer_factory,
    ];

    // When datafusion is enabled, register ArrowCacheObserverFactory so that
    // record mutations invalidate the Arrow cache for SQL query freshness.
    #[cfg(feature = "datafusion")]
    let _arrow_cache_manager = {
        let mgr = Arc::new(
            topgun_server::service::domain::arrow_cache::ArrowCacheManager::new(),
        );
        observer_factories.push(Arc::new(
            topgun_server::service::domain::arrow_cache::ArrowCacheObserverFactory::new(
                Arc::clone(&mgr),
            ),
        ));
        mgr
    };

    let record_store_factory = Arc::new(
        RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        )
        .with_observer_factories(observer_factories),
    );

    let write_validator = {
        let wv_hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            config.node_id.clone(),
            Box::new(SystemClock),
        )));
        Arc::new(WriteValidator::new(
            Arc::new(SecurityConfig::default()),
            wv_hlc,
        ))
    };

    // Arc-wrap all domain services so they can be shared across N+1 worker pipelines.
    let crdt_svc = Arc::new(CrdtService::new(
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        write_validator,
        Arc::clone(&query_registry),
        Arc::new(SchemaService::new()),
    ));
    let sync_svc = Arc::new(SyncService::new(
        merkle_manager,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
    ));
    let query_merkle_manager = Arc::new(
        topgun_server::storage::query_merkle::QueryMerkleSyncManager::new(),
    );
    let query_svc = Arc::new(QueryService::new(
        Arc::clone(&query_registry),
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        Arc::new(topgun_server::service::domain::query_backend::PredicateBackend),
        Some(Arc::clone(&query_merkle_manager)),
        config.max_query_records,
        None,
        #[cfg(feature = "datafusion")]
        None,
    ));
    let messaging_svc = Arc::new(MessagingService::new(Arc::clone(&connection_registry)));
    let coordination_svc = Arc::new(CoordinationService::new(
        cluster_state,
        Arc::clone(&connection_registry),
    ));
    let search_svc = Arc::new(SearchService::new(
        search_registry,
        search_indexes,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        search_needs_population,
    ));
    let persistence_svc = Arc::new(PersistenceService::new(
        Arc::clone(&connection_registry),
        config.node_id.clone(),
    ));

    // Factory closure: creates a fresh OperationRouter + pipeline per worker.
    // Domain services are Arc-cloned (cheap reference count bump), while
    // each worker gets its own Tower middleware stack.
    let pipeline_factory = move || {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, Arc::clone(&crdt_svc));
        router.register(service_names::SYNC, Arc::clone(&sync_svc));
        router.register(service_names::QUERY, Arc::clone(&query_svc));
        router.register(service_names::MESSAGING, Arc::clone(&messaging_svc));
        router.register(service_names::COORDINATION, Arc::clone(&coordination_svc));
        router.register(service_names::SEARCH, Arc::clone(&search_svc));
        router.register(service_names::PERSISTENCE, Arc::clone(&persistence_svc));
        build_operation_pipeline(router, &config)
    };

    let dispatch_config = DispatchConfig::default();
    let dispatcher = PartitionDispatcher::new(&dispatch_config, pipeline_factory);
    (classify_svc, dispatcher, connection_registry)
}

/// Waits for SIGTERM or SIGINT (Ctrl+C) for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
