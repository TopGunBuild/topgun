//! Test server binary for integration tests.
//!
//! Wires all 7 domain services, starts on port 0, and prints `PORT=<number>`
//! to stdout. The TS test harness reads the port from stdout to connect.
//!
//! Uses `NullDataStore` (no `PostgreSQL` dependency) and JWT secret `test-e2e-secret`
//! to match the TS test helpers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::routing::get;
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
use topgun_server::service::domain::persistence::PersistenceService;
use topgun_server::service::domain::query::{QueryMutationObserver, QueryRegistry, QueryService};
use topgun_server::service::domain::search::{
    SearchMutationObserver, SearchRegistry, SearchService, TantivyMapIndex,
};
use topgun_server::service::domain::sync::SyncService;
use topgun_server::service::middleware::build_operation_pipeline;
use topgun_server::service::operation::{service_names, OperationPipeline};
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

    let (classify_svc, pipeline, connection_registry) = build_services();

    let operation_pipeline = Arc::new(tokio::sync::Mutex::new(pipeline));

    // Build the AppState with all services wired
    let shutdown = Arc::new(ShutdownController::new());
    let state = AppState {
        registry: connection_registry,
        shutdown: Arc::clone(&shutdown),
        config: Arc::new(NetworkConfig::default()),
        start_time: Instant::now(),
        observability: None,
        operation_service: Some(classify_svc),
        operation_pipeline: Some(operation_pipeline),
        jwt_secret: Some("test-e2e-secret".to_string()),
        cluster_state: None,
        store_factory: None,
        server_config: None,
    };

    // Build the axum router with state.
    // Serve WebSocket on both /ws (integration tests) and / (browser clients).
    let ws_handler = get(topgun_server::network::handlers::ws_upgrade_handler);
    let app = axum::Router::new()
        .route("/ws", ws_handler.clone())
        .route("/", ws_handler)
        .with_state(state);

    // Use PORT env var if set (for manual testing), otherwise OS-assigned
    let bind_port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let listener = TcpListener::bind(format!("127.0.0.1:{bind_port}")).await?;
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
/// Shares the same `indexes` and `search_registry` with `SearchService` so that
/// writes indexed by the observer are immediately visible to search queries and
/// live subscriptions.
struct SearchObserverFactory {
    search_registry: Arc<SearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl ObserverFactory for SearchObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        _partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        let observer = SearchMutationObserver::new(
            map_name.to_string(),
            Arc::clone(&self.search_registry),
            Arc::clone(&self.indexes),
            Arc::clone(&self.connection_registry),
            16, // 16ms batch interval — fast enough for tests
        );
        Some(Arc::new(observer))
    }
}

/// Observer factory that creates a `QueryMutationObserver` for every map.
///
/// Shares the same `QueryRegistry` and `ConnectionRegistry` with `QueryService`
/// so that writes trigger live `QUERY_UPDATE` messages for active subscriptions.
struct QueryObserverFactory {
    query_registry: Arc<QueryRegistry>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl ObserverFactory for QueryObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        let observer = QueryMutationObserver::new(
            Arc::clone(&self.query_registry),
            Arc::clone(&self.connection_registry),
            map_name.to_string(),
            partition_id,
        );
        Some(Arc::new(observer))
    }
}

/// Wires all 7 domain services and builds the operation pipeline.
///
/// Follows the `setup()` pattern from `packages/server-rust/src/lib.rs:63-148`.
/// Returns the classifier, the boxed pipeline, and the connection registry.
#[allow(clippy::too_many_lines)]
fn build_services() -> (
    Arc<OperationService>,
    OperationPipeline,
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

    // Shared search state: indexes and registry are shared between the
    // SearchObserverFactory (write path) and SearchService (query path).
    let search_registry = Arc::new(SearchRegistry::new());
    let search_indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>> =
        Arc::new(RwLock::new(HashMap::new()));

    let search_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(SearchObserverFactory {
            search_registry: Arc::clone(&search_registry),
            indexes: Arc::clone(&search_indexes),
            connection_registry: Arc::clone(&connection_registry),
        });

    // Create query_registry early so it can be shared with both
    // QueryObserverFactory (write path) and QueryService (query path).
    let query_registry = Arc::new(QueryRegistry::new());

    let query_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(QueryObserverFactory {
            query_registry: Arc::clone(&query_registry),
            connection_registry: Arc::clone(&connection_registry),
        });

    // MerkleSyncManager must be created before RecordStoreFactory so the
    // MerkleObserverFactory can be included in with_observer_factories().
    let merkle_manager = Arc::new(MerkleSyncManager::default());

    let merkle_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(MerkleObserverFactory::new(Arc::clone(&merkle_manager)));

    let record_store_factory = Arc::new(
        RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        )
        .with_observer_factories(vec![
            search_observer_factory,
            query_observer_factory,
            merkle_observer_factory,
        ]),
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

    let mut router = OperationRouter::new();
    router.register(
        service_names::CRDT,
        Arc::new(CrdtService::new(
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
            write_validator,
        )),
    );
    router.register(
        service_names::SYNC,
        Arc::new(SyncService::new(
            merkle_manager,
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
        )),
    );
    router.register(
        service_names::QUERY,
        Arc::new(QueryService::new(
            Arc::clone(&query_registry),
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
        )),
    );
    router.register(
        service_names::MESSAGING,
        Arc::new(MessagingService::new(Arc::clone(&connection_registry))),
    );
    router.register(
        service_names::COORDINATION,
        Arc::new(CoordinationService::new(
            cluster_state,
            Arc::clone(&connection_registry),
        )),
    );
    router.register(
        service_names::SEARCH,
        Arc::new(SearchService::new(
            search_registry,
            search_indexes,
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
        )),
    );
    router.register(
        service_names::PERSISTENCE,
        Arc::new(PersistenceService::new(
            Arc::clone(&connection_registry),
            config.node_id.clone(),
        )),
    );

    let pipeline = build_operation_pipeline(router, &config);
    (classify_svc, pipeline, connection_registry)
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
