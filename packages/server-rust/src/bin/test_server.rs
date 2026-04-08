//! Test server binary for integration tests.
//!
//! Wires all 7 domain services, starts on port 0, and prints `PORT=<number>`
//! to stdout. The TS test harness reads the port from stdout to connect.
//!
//! Uses `NullDataStore` (no `PostgreSQL` dependency) and JWT secret `test-e2e-secret`
//! to match the TS test helpers.
//!
//! Optional cluster mode: when `--seed-nodes` is provided, the server participates
//! in cluster formation, heartbeat-based failure detection, and partition rebalancing.
//! Running without `--seed-nodes` preserves the original single-node behavior.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use axum::routing::{delete, get, post};
use clap::Parser;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{mpsc, watch};
use topgun_core::{SystemClock, HLC};

use topgun_server::cluster::failure_detector::PhiAccrualConfig;
use topgun_server::cluster::peer_connection::PeerConnectionMap;
use topgun_server::cluster::state::{ClusterState, InboundClusterMessage, MigrationCommand};
use topgun_server::cluster::types::{ClusterConfig, MemberInfo, NodeState};
use topgun_server::cluster::{ClusterFormationService, HeartbeatService, MembershipReactor, PhiAccrualFailureDetector};
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
use topgun_server::service::policy::{InMemoryPolicyStore, PolicyEvaluator, PolicyStore};
use topgun_server::service::security::{SecurityConfig, WriteValidator};
use topgun_server::service::OperationService;
use topgun_server::storage::datastores::NullDataStore;
use topgun_server::storage::factory::{ObserverFactory, RecordStoreFactory};
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use topgun_server::storage::mutation_observer::MutationObserver;

// ---------------------------------------------------------------------------
// CLI argument definition
// ---------------------------------------------------------------------------

/// Test server for integration tests and multi-node cluster validation.
///
/// When `--seed-nodes` is empty, the server runs as a standalone single-node
/// instance (backward-compatible with existing integration tests). When
/// `--seed-nodes` lists peer addresses, the server participates in cluster
/// formation, heartbeat failure detection, and partition rebalancing.
#[derive(Parser, Debug)]
#[command(name = "test-server")]
struct Args {
    /// Unique identifier for this node in the cluster.
    #[arg(long, default_value = "test-server-node")]
    node_id: String,

    /// Host name or IP that peers use to reach this node's cluster port.
    /// Also used as the `MemberInfo.host` in join handshakes.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Client WebSocket port. Reads PORT env var if not set; 0 means OS-assigned.
    #[arg(long, env = "PORT", default_value_t = 0)]
    port: u16,

    /// Inter-node cluster TCP port. 0 means OS-assigned.
    #[arg(long, default_value_t = 0)]
    cluster_port: u16,

    /// Comma-separated list of seed node addresses for cluster formation.
    /// Example: "127.0.0.1:11001,127.0.0.1:11002". Empty string = single-node mode.
    #[arg(long, default_value = "")]
    seed_nodes: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let node_id = args.node_id.clone();

    // Bind the client WebSocket listener first so we know the actual port.
    // Bind to all interfaces so inter-container traffic (Docker networking) reaches
    // the server. TOPGUN_BIND_ADDR overrides the default for environments that
    // require loopback-only binding.
    let bind_addr = std::env::var("TOPGUN_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let listener = TcpListener::bind(format!("{bind_addr}:{}", args.port)).await?;
    let bound_port = listener.local_addr()?.port();

    // Parse seed nodes, filtering empty strings produced by splitting an empty value.
    let seed_list: Vec<String> = args
        .seed_nodes
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let cluster_mode = !seed_list.is_empty();

    // Create a shutdown watch channel used to coordinate graceful termination of
    // cluster background services (HeartbeatService, etc.).
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Create the policy store early so it can be shared between the admin HTTP API
    // (AppState.policy_store) and the authorization middleware (PolicyEvaluator).
    // Both hold Arc references to the same underlying InMemoryPolicyStore, so
    // policies created via the admin API are immediately visible to the evaluator.
    let policy_store = Arc::new(InMemoryPolicyStore::new());
    let policy_evaluator = Arc::new(PolicyEvaluator::new(
        policy_store.clone() as Arc<dyn PolicyStore>,
    ));

    let (cluster_state_for_services, cluster_state_for_app) = if cluster_mode {
        // --- Cluster mode ---

        // Bind the inter-node TCP listener on the cluster port.
        let cluster_listener =
            TcpListener::bind(format!("0.0.0.0:{}", args.cluster_port)).await?;
        let bound_cluster_port = cluster_listener.local_addr()?.port();

        let cluster_config = Arc::new(ClusterConfig {
            seed_addresses: seed_list,
            ..ClusterConfig::default()
        });

        let (cs, change_rx) =
            ClusterState::new(Arc::clone(&cluster_config), node_id.clone());
        let cluster_state = Arc::new(cs);

        let peers = Arc::new(PeerConnectionMap::new());

        // The inbound message channel feeds non-formation frames (heartbeats, DAG ops)
        // from the per-peer read loops into the routing layer.
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<InboundClusterMessage>();

        // Heartbeat-specific inbound channel: a routing task selects heartbeat and
        // complaint frames out of the shared inbound stream and forwards them here.
        let (heartbeat_tx, heartbeat_rx) = mpsc::unbounded_channel::<topgun_server::cluster::ClusterMessage>();

        // Spawn the routing task that fans out inbound frames to the heartbeat channel.
        // All other variants are currently discarded; future dispatch integration will
        // route them to the cluster dispatch loop instead.
        tokio::spawn(async move {
            let mut rx = inbound_rx;
            while let Some(msg) = rx.recv().await {
                match msg.message {
                    topgun_server::cluster::ClusterMessage::Heartbeat(_)
                    | topgun_server::cluster::ClusterMessage::HeartbeatComplaint(_) => {
                        let _ = heartbeat_tx.send(msg.message);
                    }
                    _ => {
                        // Non-heartbeat frames: no dispatch service wired yet.
                    }
                }
            }
        });

        // Migration command channel; the receiver is retained until a dedicated
        // migration service is wired in a follow-up spec.
        let (migration_tx, _migration_rx) = mpsc::channel::<MigrationCommand>(64);

        // Build the local member descriptor used in the join handshake.
        let local_member = MemberInfo {
            node_id: node_id.clone(),
            host: args.host.clone(),
            client_port: bound_port,
            cluster_port: bound_cluster_port,
            state: NodeState::Joining,
            join_version: 0,
        };

        // Wire up the cluster formation service (seed discovery + inbound listener).
        let formation_svc = ClusterFormationService::new(
            Arc::clone(&cluster_state),
            Arc::clone(&peers),
            Arc::clone(&cluster_config),
            local_member,
            inbound_tx,
        );
        Arc::new(formation_svc).start(cluster_listener);

        // Wire up the heartbeat service (phi-accrual failure detection).
        let failure_detector = Arc::new(PhiAccrualFailureDetector::new(PhiAccrualConfig {
            phi_threshold: cluster_config.phi_threshold,
            max_sample_size: cluster_config.max_sample_size,
            min_std_dev_ms: cluster_config.min_std_dev_ms,
            max_no_heartbeat_ms: cluster_config.max_no_heartbeat_ms,
            heartbeat_interval_ms: cluster_config.heartbeat_interval_ms,
        }));
        let heartbeat_svc = HeartbeatService {
            cluster_state: Arc::clone(&cluster_state),
            peers: Arc::clone(&peers),
            failure_detector,
            config: Arc::clone(&cluster_config),
            suspected_at: DashMap::new(),
        };
        tokio::spawn(Arc::new(heartbeat_svc).run(heartbeat_rx, shutdown_rx));

        // Build domain services, sharing the cluster_state with CoordinationService.
        // Pass the policy evaluator so every client operation is checked against RBAC.
        let (classify_svc, dispatcher, connection_registry) =
            build_services(node_id.clone(), Arc::clone(&cluster_state), Some(Arc::clone(&policy_evaluator)));

        // Wire up the membership reactor (partition rebalancing on member changes).
        let reactor = MembershipReactor {
            cluster_state: Arc::clone(&cluster_state),
            peers,
            connection_registry: Arc::clone(&connection_registry),
            config: cluster_config,
            migration_tx,
        };
        tokio::spawn(Arc::new(reactor).run(change_rx));

        (
            (classify_svc, dispatcher, connection_registry),
            Some(cluster_state),
        )
    } else {
        // --- Single-node mode (backward-compatible) ---
        let (cs, _rx) = ClusterState::new(
            Arc::new(ClusterConfig::default()),
            node_id.clone(),
        );
        let cs = Arc::new(cs);
        let (classify_svc, dispatcher, connection_registry) =
            build_services(node_id.clone(), Arc::clone(&cs), Some(Arc::clone(&policy_evaluator)));

        // Single-node mode: expose no cluster state to AppState (existing behavior).
        ((classify_svc, dispatcher, connection_registry), None)
    };

    let (classify_svc, dispatcher, connection_registry) = cluster_state_for_services;

    // Allow integration test environments to opt in to detailed auth errors.
    let insecure_forward_auth_errors = std::env::var("INSECURE_FORWARD_AUTH_ERRORS")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    let shutdown = Arc::new(ShutdownController::new());
    let state = AppState {
        registry: connection_registry,
        shutdown: Arc::clone(&shutdown),
        config: Arc::new(NetworkConfig {
            insecure_forward_auth_errors,
            ..NetworkConfig::default()
        }),
        start_time: Instant::now(),
        observability: None,
        operation_service: Some(classify_svc),
        dispatcher: Some(Arc::new(dispatcher)),
        jwt_secret: Some("test-e2e-secret".to_string()),
        cluster_state: cluster_state_for_app,
        store_factory: None,
        server_config: None,
        policy_store: Some(policy_store),
        auth_providers: Arc::new(vec![]),
        refresh_grant_store: None,
        auth_validator: None,
        index_observer_factory: None,
        backfill_progress: Arc::new(dashmap::DashMap::new()),
    };

    // Build the axum router with state.
    // Serve WebSocket on both /ws (integration tests) and / (browser clients).
    // Include /health so Docker Compose healthchecks and inter-container probes succeed.
    // Mount admin policy routes so integration tests can create policies via HTTP.
    let ws_handler = get(topgun_server::network::handlers::ws_upgrade_handler);
    let health_handler = get(topgun_server::network::handlers::health_handler);
    let app = axum::Router::new()
        .route("/ws", ws_handler.clone())
        .route("/", ws_handler)
        .route("/health", health_handler)
        .route(
            "/api/admin/policies",
            get(topgun_server::network::handlers::admin::list_policies)
                .post(topgun_server::network::handlers::admin::create_policy),
        )
        .route(
            "/api/admin/policies/{id}",
            delete(topgun_server::network::handlers::admin::delete_policy),
        )
        .route(
            "/sync",
            post(topgun_server::network::handlers::http_sync_handler),
        )
        .with_state(state);

    // Print port to stdout so the TS test harness can read it
    println!("PORT={bound_port}");

    // Mark the server as ready
    shutdown.set_ready();

    // Serve until SIGTERM or SIGINT; signal cluster services to shut down.
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            // Signal cluster services (e.g. HeartbeatService) to exit their loops.
            let _ = shutdown_tx.send(true);
        })
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Observer factory
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Service wiring
// ---------------------------------------------------------------------------

/// Wires all 7 domain services and builds the partition dispatcher.
///
/// Accepts `node_id`, `cluster_state`, and an optional `PolicyEvaluator` so
/// callers can wire RBAC enforcement into the Tower middleware pipeline.
/// When `policy_evaluator` is `Some`, every client operation is checked against
/// the policy store before reaching domain services.
///
/// The `connection_registry` returned must be shared with `AppState.registry`
/// so domain services can track subscriptions and route messages by `connection_id`.
///
/// Follows the `setup()` pattern from `packages/server-rust/src/lib.rs`.
/// Domain services are `Arc`-wrapped and shared across all worker pipelines.
/// Each worker gets its own `OperationRouter` + `OperationPipeline` via a
/// factory closure passed to `PartitionDispatcher::new()`.
#[allow(clippy::too_many_lines)]
fn build_services(
    node_id: String,
    cluster_state: Arc<ClusterState>,
    policy_evaluator: Option<Arc<PolicyEvaluator>>,
) -> (
    Arc<OperationService>,
    PartitionDispatcher,
    Arc<ConnectionRegistry>,
) {
    let config = ServerConfig {
        node_id: node_id.clone(),
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
        node_id,
    ));

    // Factory closure: creates a fresh OperationRouter + pipeline per worker.
    // Domain services are Arc-cloned (cheap reference count bump), while
    // each worker gets its own Tower middleware stack.
    // When a PolicyEvaluator is provided, it is passed to build_operation_pipeline
    // so every client operation is checked against the RBAC policy store.
    let evaluator_for_factory = policy_evaluator;
    let pipeline_factory = move || {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, Arc::clone(&crdt_svc));
        router.register(service_names::SYNC, Arc::clone(&sync_svc));
        router.register(service_names::QUERY, Arc::clone(&query_svc));
        router.register(service_names::MESSAGING, Arc::clone(&messaging_svc));
        router.register(service_names::COORDINATION, Arc::clone(&coordination_svc));
        router.register(service_names::SEARCH, Arc::clone(&search_svc));
        router.register(service_names::PERSISTENCE, Arc::clone(&persistence_svc));
        build_operation_pipeline(
            router,
            &config,
            evaluator_for_factory.clone(),
        )
    };

    let dispatch_config = DispatchConfig::default();
    let dispatcher = PartitionDispatcher::new(&dispatch_config, pipeline_factory);
    (classify_svc, dispatcher, connection_registry)
}

// ---------------------------------------------------------------------------
// Shutdown signal
// ---------------------------------------------------------------------------

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
