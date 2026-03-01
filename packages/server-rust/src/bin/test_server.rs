//! Test server binary for integration tests.
//!
//! Wires all 7 domain services, starts on port 0, and prints `PORT=<number>`
//! to stdout. The TS test harness reads the port from stdout to connect.
//!
//! Uses `NullDataStore` (no `PostgreSQL` dependency) and JWT secret `test-e2e-secret`
//! to match the TS test helpers.

use std::sync::Arc;
use std::time::Instant;

use axum::routing::get;
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
use topgun_server::service::domain::query::{QueryRegistry, QueryService};
use topgun_server::service::domain::search::{SearchRegistry, SearchService};
use topgun_server::service::domain::sync::SyncService;
use topgun_server::service::middleware::build_operation_pipeline;
use topgun_server::service::operation::{service_names, OperationPipeline};
use topgun_server::service::router::OperationRouter;
use topgun_server::service::security::{SecurityConfig, WriteValidator};
use topgun_server::service::OperationService;
use topgun_server::storage::datastores::NullDataStore;
use topgun_server::storage::factory::RecordStoreFactory;
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::merkle_sync::MerkleSyncManager;

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
    };

    // Build the axum router with state
    let app = axum::Router::new()
        .route(
            "/ws",
            get(topgun_server::network::handlers::ws_upgrade_handler),
        )
        .with_state(state);

    // Bind to port 0 (OS-assigned ephemeral port)
    let listener = TcpListener::bind("127.0.0.1:0").await?;
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

/// Wires all 7 domain services and builds the operation pipeline.
///
/// Follows the `setup()` pattern from `packages/server-rust/src/lib.rs:63-148`.
/// Returns the classifier, the boxed pipeline, and the connection registry.
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

    let record_store_factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        Arc::new(NullDataStore),
        Vec::new(),
    ));
    let merkle_manager = Arc::new(MerkleSyncManager::default());

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
    let query_registry = Arc::new(QueryRegistry::new());
    router.register(
        service_names::QUERY,
        Arc::new(QueryService::new(
            query_registry,
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
            Arc::new(SearchRegistry::new()),
            Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
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
