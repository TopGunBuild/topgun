//! Test server binary for integration tests.
//!
//! Wires all 7 domain services, starts on the configured port (default 8080),
//! and prints `PORT=<number>` to stdout. The TS test harness passes `--port 0`
//! to opt into an ephemeral port and reads it from stdout to connect.
//!
//! Uses `NullDataStore` (no `PostgreSQL` dependency). The JWT signing secret is
//! read from the `JWT_SECRET` env var; the process fails fast if it is unset and
//! `TOPGUN_NO_AUTH` is not `1` / `true`. Integration tests, benches, and the
//! sync-lab demo expect `JWT_SECRET=test-e2e-secret` (matched against the
//! pre-signed tokens in `tests/integration-rust/helpers/test-client.ts` and the
//! demo token in `examples/sync-lab/src/lib/device-manager.ts`).
//!
//! Optional cluster mode: when `--seed-nodes` is provided, the server participates
//! in cluster formation, heartbeat-based failure detection, and partition rebalancing.
//! Running without `--seed-nodes` preserves the original single-node behavior.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use axum::routing::get;
use clap::Parser;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{mpsc, oneshot, watch};
use topgun_core::{SystemClock, HLC};

use topgun_server::cluster::failure_detector::PhiAccrualConfig;
use topgun_server::cluster::messages::DagCompletePayload;
use topgun_server::cluster::peer_connection::PeerConnectionMap;
use topgun_server::cluster::state::{ClusterState, InboundClusterMessage, MigrationCommand};
use topgun_server::cluster::types::{
    ClusterConfig, ClusterHealth, MemberInfo, MembersView, NodeState,
};
use topgun_server::cluster::{
    run_cluster_dispatch_loop, ClusterChange, ClusterDispatchContext, ClusterFormationService,
    ClusterPartitionTable, ClusterService, HeartbeatService, MembershipReactor,
    PhiAccrualFailureDetector,
};
use topgun_server::dag::types::QueryConfig;
use topgun_server::dag::ClusterQueryCoordinator;
use topgun_server::network::config::NetworkConfig;
use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::network::handlers::AppState;
use topgun_server::network::shutdown::ShutdownController;
use topgun_server::service::config::ServerConfig;
use topgun_server::service::dispatch::{DispatchConfig, PartitionDispatcher};
use topgun_server::service::domain::coordination::CoordinationService;
use topgun_server::service::domain::counter::CounterRegistry;
use topgun_server::service::domain::crdt::CrdtService;
use topgun_server::service::domain::embedding::{
    noop::NoopEmbeddingProvider, EmbeddingConfig, EmbeddingObserverFactory, EmbeddingProvider,
    EmbeddingProviderConfig, NoopConfig, VectorConfig as EmbeddingVectorConfig,
};
use topgun_server::service::domain::index::IndexObserverFactory;
use topgun_server::service::domain::messaging::{MessagingService, TopicRegistry};
use topgun_server::service::domain::persistence::PersistenceService;
use topgun_server::service::domain::query::{QueryRegistry, QueryService};
use topgun_server::service::domain::schema::SchemaService;
use topgun_server::service::domain::search::{
    HybridSearchRegistry, SearchConfig, SearchMutationObserver, SearchRegistry, SearchService,
    TantivyMapIndex,
};
use topgun_server::service::domain::sync::SyncService;
use topgun_server::service::domain::LockRegistry;
use topgun_server::service::middleware::build_operation_pipeline;
use topgun_server::service::operation::service_names;
use topgun_server::service::policy::{InMemoryPolicyStore, PolicyEvaluator, PolicyStore};
use topgun_server::service::registry::{ManagedService, ServiceContext};
use topgun_server::service::router::OperationRouter;
use topgun_server::service::security::{SecurityConfig, WriteValidator};
use topgun_server::service::OperationService;
#[cfg(feature = "redb")]
use topgun_server::storage::datastores::RedbDataStore;
use topgun_server::storage::datastores::{NullDataStore, WriteBehindConfig, WriteBehindDataStore};
use topgun_server::storage::eviction_config::EvictionConfig;
use topgun_server::storage::eviction_orchestrator::EvictionOrchestrator;
use topgun_server::storage::factory::{ObserverFactory, RecordStoreFactory};
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use topgun_server::storage::mutation_observer::MutationObserver;

// ---------------------------------------------------------------------------
// Storage backend selection
// ---------------------------------------------------------------------------

/// Identifies the active persistence backend so the bootstrap can decide
/// whether to wrap the data store with `WriteBehindDataStore` (skipped for the
/// `Null` backend because buffering writes to a no-op store wastes overhead).
#[derive(Debug, Clone, Copy)]
enum StorageBackend {
    Null,
    Redb,
    // Constructed only under `feature = "postgres"`; the variant is still listed
    // here so the bootstrap match arms compile in every feature configuration.
    #[allow(dead_code)]
    Postgres,
}

/// Resolve the storage backend at startup.
///
/// Reads `STORAGE_BACKEND` (default: `redb`) and constructs the matching
/// `Arc<dyn MapDataStore>`. Each branch is responsible for any per-backend
/// initialization (e.g. opening a redb file, connecting to Postgres).
///
/// Returns the constructed store paired with a `StorageBackend` discriminant
/// so the bootstrap can branch on backend variant when deciding whether to
/// wrap with `WriteBehindDataStore` without re-reading the env var.
///
/// The default is `redb` so a developer running `pnpm start:server` with no
/// env vars gets durable embedded storage out of the box. Set
/// `STORAGE_BACKEND=null` for ephemeral integration-test fixtures, or
/// `STORAGE_BACKEND=postgres` (with the `postgres` feature compiled in) for
/// production deployments.
async fn select_datastore() -> anyhow::Result<(Arc<dyn MapDataStore>, StorageBackend)> {
    let backend = std::env::var("STORAGE_BACKEND").unwrap_or_else(|_| "redb".to_string());
    match backend.as_str() {
        "null" => Ok((Arc::new(NullDataStore), StorageBackend::Null)),
        #[cfg(feature = "redb")]
        "redb" => {
            let path =
                std::env::var("TOPGUN_REDB_PATH").unwrap_or_else(|_| "./topgun.redb".to_string());
            let store = RedbDataStore::new(&path)?;
            store.initialize().await?;
            Ok((Arc::new(store), StorageBackend::Redb))
        }
        #[cfg(not(feature = "redb"))]
        "redb" => anyhow::bail!(
            "STORAGE_BACKEND=redb requested but topgun-server was built without the `redb` feature"
        ),
        #[cfg(feature = "postgres")]
        "postgres" => {
            let url = std::env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("STORAGE_BACKEND=postgres requires DATABASE_URL"))?;
            let pool = sqlx::PgPool::connect(&url).await?;
            let store = topgun_server::storage::datastores::PostgresDataStore::new(pool, None)?;
            store.initialize().await?;
            Ok((Arc::new(store), StorageBackend::Postgres))
        }
        #[cfg(not(feature = "postgres"))]
        "postgres" => anyhow::bail!(
            "STORAGE_BACKEND=postgres requested but topgun-server was built without the `postgres` feature"
        ),
        other => anyhow::bail!(
            "Unknown STORAGE_BACKEND='{other}' (expected: redb, postgres, null)"
        ),
    }
}

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
#[command(name = "topgun-server")]
struct Args {
    /// Unique identifier for this node in the cluster.
    #[arg(long, default_value = "topgun-server-node")]
    node_id: String,

    /// Host name or IP that peers use to reach this node's cluster port.
    /// Also used as the `MemberInfo.host` in join handshakes.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Client WebSocket port. Reads PORT env var if not set; 0 means OS-assigned.
    /// Default 8080 matches every documented quick-start surface (README, intro.mdx,
    /// quick-start.mdx, mcp-server.mdx, Hero snippet).
    #[arg(long, env = "PORT", default_value_t = 8080)]
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
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when `addr` is a loopback address.
///
/// Treats `127.0.0.1`, `::1`, and any value starting with `localhost` as
/// loopback. All other values (including `0.0.0.0`) are considered non-loopback
/// and will trigger the no-auth + exposed-bind warning.
fn is_loopback(addr: &str) -> bool {
    addr == "127.0.0.1" || addr == "::1" || addr.starts_with("localhost")
}

// ---------------------------------------------------------------------------
// ClusterStateAdapter — bridges ClusterState to the ClusterService trait
// ---------------------------------------------------------------------------

/// Thin adapter so `ClusterState` can satisfy `Arc<dyn ClusterService>` for
/// the `ClusterQueryCoordinator`.  Only the production binary needs this;
/// simulation tests use `SimClusterService` directly.
struct ClusterStateAdapter {
    state: Arc<ClusterState>,
    node_id: String,
}

#[async_trait]
impl ManagedService for ClusterStateAdapter {
    fn name(&self) -> &'static str {
        "cluster-state-adapter"
    }
    async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
        Ok(())
    }
    async fn reset(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

impl ClusterService for ClusterStateAdapter {
    fn node_id(&self) -> &str {
        &self.node_id
    }
    fn is_master(&self) -> bool {
        self.state.is_master()
    }
    fn master_id(&self) -> Option<String> {
        let view = self.state.current_view();
        view.members.first().map(|m| m.node_id.clone())
    }
    fn members_view(&self) -> Arc<MembersView> {
        self.state.current_view()
    }
    fn partition_table(&self) -> &ClusterPartitionTable {
        &self.state.partition_table
    }
    fn subscribe_changes(&self) -> tokio::sync::mpsc::UnboundedReceiver<ClusterChange> {
        // Callers that need change events should use ClusterState::change_sender() directly;
        // this adapter is used only by ClusterQueryCoordinator, which polls membership
        // on each execute_distributed call and does not subscribe to change events.
        tokio::sync::mpsc::unbounded_channel().1
    }
    fn health(&self) -> ClusterHealth {
        let view = self.state.current_view();
        ClusterHealth {
            node_count: view.members.len(),
            active_nodes: view.members.len(),
            suspect_nodes: 0,
            partition_table_version: self.state.partition_table.version(),
            active_migrations: 0,
            is_master: self.state.is_master(),
            master_node_id: self.master_id(),
        }
    }
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

    // When set to 1 or true, omit the JWT secret so templates and QA harness
    // can connect without auth tokens.
    let no_auth = std::env::var("TOPGUN_NO_AUTH")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    // Bind the client WebSocket listener first so we know the actual port.
    // The default interface depends on auth posture: with auth disabled
    // (TOPGUN_NO_AUTH=1) we bind loopback-only so a zero-config local server is
    // never inadvertently exposed to the network. With auth enforced we bind all
    // interfaces so inter-container traffic (Docker networking) reaches the server.
    // TOPGUN_BIND_ADDR always overrides this default.
    let default_bind = if no_auth { "127.0.0.1" } else { "0.0.0.0" };
    let bind_addr = std::env::var("TOPGUN_BIND_ADDR").unwrap_or_else(|_| default_bind.to_string());

    // When no-auth is active but the operator has overridden the bind address to
    // a non-loopback interface, the admin control plane (/api/admin/*) becomes
    // reachable unauthenticated from the network. Warn loudly so the operator
    // can take corrective action; the bind itself is not altered here.
    if no_auth && !is_loopback(&bind_addr) {
        tracing::warn!(
            bind_addr = %bind_addr,
            "TOPGUN_NO_AUTH=1 with a non-loopback bind address: /api/admin/* endpoints \
             are reachable unauthenticated on a network interface. Set TOPGUN_BIND_ADDR=127.0.0.1 \
             or enable JWT authentication to restrict access."
        );
    }

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

    // Clone the shutdown receiver for the eviction orchestrator before any of the
    // cluster background services move `shutdown_rx`. Cluster-mode HeartbeatService
    // spawn (below) consumes `shutdown_rx` by value; without an early clone the
    // orchestrator spawn would fail to compile with use-of-moved-value.
    let orchestrator_shutdown = shutdown_rx.clone();

    // Resolve eviction + write-behind tunables from the operator's environment as
    // early as possible so the bootstrap can branch on backend variant when wrapping
    // the data store and emit a single boot-time summary log without duplicating
    // env-var reads later in the function.
    let eviction_config = EvictionConfig::from_env();
    let write_behind_config = WriteBehindConfig::from_env();

    // Create the policy store early so it can be shared between the admin HTTP API
    // (AppState.policy_store) and the authorization middleware (PolicyEvaluator).
    // Both hold Arc references to the same underlying InMemoryPolicyStore, so
    // policies created via the admin API are immediately visible to the evaluator.
    let policy_store = Arc::new(InMemoryPolicyStore::new());
    let policy_evaluator = Arc::new(PolicyEvaluator::new(
        policy_store.clone() as Arc<dyn PolicyStore>
    ));

    // Resolve the persistence backend once per process. The default branch
    // creates ./topgun.redb in the working directory; STORAGE_BACKEND=null
    // preserves the legacy ephemeral integration-test behavior.
    let (inner_data_store, backend) = select_datastore().await?;

    // Wrap durable backends in WriteBehindDataStore so client OP_ACK round-trips
    // do not block on disk fsync. The Null backend is intentionally skipped:
    // buffering writes to a no-op store wastes overhead with no durability win.
    let datastore: Arc<dyn MapDataStore> = match backend {
        StorageBackend::Redb | StorageBackend::Postgres => {
            tracing::debug!(
                target: "topgun_server::bootstrap",
                backend = ?backend,
                "write-behind wrap applied"
            );
            // WriteBehindDataStore::new returns Arc<Self> directly (it spawns its
            // own background flush task). Wrapping with Arc::new here would
            // produce Arc<Arc<...>>, which cannot coerce to Arc<dyn MapDataStore>.
            WriteBehindDataStore::new(inner_data_store, write_behind_config.clone())
        }
        StorageBackend::Null => {
            tracing::debug!(
                target: "topgun_server::bootstrap",
                "write-behind skipped for null backend"
            );
            inner_data_store
        }
    };

    let (cluster_state_for_services, cluster_state_for_app) = if cluster_mode {
        // --- Cluster mode ---

        // Bind the inter-node TCP listener on the cluster port.
        let cluster_listener = TcpListener::bind(format!("0.0.0.0:{}", args.cluster_port)).await?;
        let bound_cluster_port = cluster_listener.local_addr()?.port();

        let cluster_config = Arc::new(ClusterConfig {
            seed_addresses: seed_list,
            ..ClusterConfig::default()
        });

        let (cs, change_rx) = ClusterState::new(Arc::clone(&cluster_config), node_id.clone());
        let cluster_state = Arc::new(cs);

        let peers = Arc::new(PeerConnectionMap::new());

        // The inbound message channel feeds non-formation frames (heartbeats, DAG ops)
        // from the per-peer read loops into the routing layer.
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<InboundClusterMessage>();

        // Heartbeat-specific inbound channel: a routing task selects heartbeat and
        // complaint frames out of the shared inbound stream and forwards them here.
        let (heartbeat_tx, heartbeat_rx) =
            mpsc::unbounded_channel::<topgun_server::cluster::ClusterMessage>();

        // The completion_registry is shared between the ClusterQueryCoordinator
        // (which inserts oneshot senders before fanning out) and the cluster
        // dispatch loop (which resolves them when DagComplete frames arrive from
        // peer nodes).  Both must hold a reference to the SAME Arc or the
        // coordinator will time out waiting for completions that are never resolved.
        let completion_registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>> =
            Arc::new(DashMap::new());

        // Dispatch channel: the routing task forwards DAG frames here; the
        // dispatch loop consumes them and routes to the appropriate handler.
        let (dispatch_tx, dispatch_rx) = mpsc::channel::<InboundClusterMessage>(1024);

        // Spawn the routing task that fans out inbound frames.  Heartbeat and
        // complaint frames go to the heartbeat service; DAG frames (DagExecute,
        // DagComplete, DagData) go to the cluster dispatch loop so the coordinator
        // can receive peer completions.  All other variants are logged and dropped.
        tokio::spawn(async move {
            let mut rx = inbound_rx;
            while let Some(msg) = rx.recv().await {
                match &msg.message {
                    topgun_server::cluster::ClusterMessage::Heartbeat(_)
                    | topgun_server::cluster::ClusterMessage::HeartbeatComplaint(_) => {
                        let _ = heartbeat_tx.send(msg.message);
                    }
                    topgun_server::cluster::ClusterMessage::DagExecute(_)
                    | topgun_server::cluster::ClusterMessage::DagComplete(_)
                    | topgun_server::cluster::ClusterMessage::DagData(_) => {
                        // Route DAG frames to the dispatch loop so DagComplete can
                        // resolve the coordinator's awaiting oneshot receivers.
                        let _ = dispatch_tx.send(msg).await;
                    }
                    _ => {
                        tracing::trace!(
                            "inbound cluster frame not routed (no handler wired): {:?}",
                            std::mem::discriminant(&msg.message)
                        );
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
        // Pass the completion_registry and cluster_state so build_services can
        // construct the ClusterQueryCoordinator with the real connection_registry
        // and record_store_factory that it creates internally.
        let (
            classify_svc,
            dispatcher,
            connection_registry,
            lock_registry,
            topic_registry,
            counter_registry,
            record_store_factory,
            index_observer_factory,
        ) = build_services(
            node_id.clone(),
            Arc::clone(&cluster_state),
            Some(Arc::clone(&policy_evaluator)),
            Arc::clone(&datastore),
            Some((Arc::clone(&cluster_state), Arc::clone(&completion_registry))),
        );

        // Wire up the membership reactor (partition rebalancing on member changes).
        let reactor = MembershipReactor {
            cluster_state: Arc::clone(&cluster_state),
            peers,
            connection_registry: Arc::clone(&connection_registry),
            config: cluster_config,
            migration_tx,
        };
        tokio::spawn(Arc::new(reactor).run(change_rx));

        // Spawn the cluster dispatch loop.  It consumes DAG frames forwarded by
        // the routing task above and routes them to the appropriate handlers.
        // The completion_registry Arc is shared with the coordinator (wired inside
        // build_services) so DagComplete frames resolve the coordinator's receivers.
        let dispatch_ctx = ClusterDispatchContext {
            local_node_id: node_id.clone(),
            completion_registry: Arc::clone(&completion_registry),
            record_store_factory: Arc::clone(&record_store_factory),
            connection_registry: Arc::clone(&connection_registry),
        };
        tokio::spawn(run_cluster_dispatch_loop(dispatch_ctx, dispatch_rx));

        (
            (
                classify_svc,
                dispatcher,
                connection_registry,
                lock_registry,
                topic_registry,
                counter_registry,
                record_store_factory,
                index_observer_factory,
            ),
            Some(cluster_state),
        )
    } else {
        // --- Single-node mode (backward-compatible) ---
        let (cs, _rx) = ClusterState::new(Arc::new(ClusterConfig::default()), node_id.clone());
        let cs = Arc::new(cs);
        let (
            classify_svc,
            dispatcher,
            connection_registry,
            lock_registry,
            topic_registry,
            counter_registry,
            record_store_factory,
            index_observer_factory,
        ) = build_services(
            node_id.clone(),
            Arc::clone(&cs),
            Some(Arc::clone(&policy_evaluator)),
            Arc::clone(&datastore),
            None,
        );

        // Single-node mode: expose no cluster state to AppState (existing behavior).
        (
            (
                classify_svc,
                dispatcher,
                connection_registry,
                lock_registry,
                topic_registry,
                counter_registry,
                record_store_factory,
                index_observer_factory,
            ),
            None,
        )
    };

    let (
        classify_svc,
        dispatcher,
        connection_registry,
        lock_registry,
        topic_registry,
        counter_registry,
        record_store_factory,
        index_observer_factory,
    ) = cluster_state_for_services;

    // Spawn the eviction orchestrator after services are wired so it observes
    // every store the factory will create. The orchestrator terminates within
    // one `interval_ms` of `shutdown_tx.send(true)` via the cloned receiver
    // captured at the top of `main`.
    let orchestrator = EvictionOrchestrator::new(
        eviction_config.clone(),
        Arc::clone(&record_store_factory),
        orchestrator_shutdown,
    );
    tokio::spawn(orchestrator.run());

    // Single operator-facing summary line. Reads against this line are how an
    // operator confirms the effective ceiling and whether write-behind is
    // wrapping the durable backend without reading source.
    tracing::info!(
        max_ram_mb = eviction_config.max_ram_bytes / (1024 * 1024),
        high_water_pct = eviction_config.high_water_pct,
        low_water_pct = eviction_config.low_water_pct,
        interval_ms = eviction_config.interval_ms,
        write_behind_enabled = !matches!(backend, StorageBackend::Null),
        write_behind_shutdown_timeout_ms = write_behind_config.shutdown_timeout_ms,
        "eviction + write-behind initialized"
    );

    // Allow integration test environments to opt in to detailed auth errors.
    let insecure_forward_auth_errors = std::env::var("INSECURE_FORWARD_AUTH_ERRORS")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    // Comma-separated list of allowed CORS origins. Empty (the default) rejects
    // all cross-origin browser requests, which is the safe default but means
    // browser SDKs hosted on a different origin than the server cannot connect
    // until this is set. Operators set TOPGUN_CORS_ORIGINS to enable.
    let cors_origins = std::env::var("TOPGUN_CORS_ORIGINS")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Read the JWT signing secret from env. We refuse to start with a baked-in
    // secret because earlier revisions of this binary defaulted to a publicly-
    // known value, which silently turned any deployment of the prod Docker
    // image into a forge-anybody's-tokens vulnerability whenever the operator
    // forgot to override.
    let jwt_secret = if no_auth {
        None
    } else {
        match std::env::var("JWT_SECRET") {
            Ok(s) if !s.is_empty() => Some(s),
            _ => {
                eprintln!(
                    "ERROR: JWT_SECRET environment variable must be set when auth is enabled.\n  \
                     For production:   JWT_SECRET=$(openssl rand -base64 32) <command>\n  \
                     For local dev:    TOPGUN_NO_AUTH=1 <command>\n  \
                     For integration tests + sync-lab demo: JWT_SECRET=test-e2e-secret <command>"
                );
                std::process::exit(1);
            }
        }
    };

    let shutdown = Arc::new(ShutdownController::new());
    let state = AppState {
        registry: connection_registry,
        shutdown: Arc::clone(&shutdown),
        config: Arc::new(NetworkConfig {
            insecure_forward_auth_errors,
            cors_origins,
            ..NetworkConfig::default()
        }),
        start_time: Instant::now(),
        observability: None,
        operation_service: Some(classify_svc),
        dispatcher: Some(Arc::new(dispatcher)),
        jwt_secret,
        cluster_state: cluster_state_for_app,
        store_factory: Some(Arc::clone(&record_store_factory)),
        server_config: Some(Arc::new(ArcSwap::from_pointee(ServerConfig {
            node_id: node_id.clone(),
            ..ServerConfig::default()
        }))),
        policy_store: Some(policy_store),
        auth_providers: Arc::new(vec![]),
        refresh_grant_store: None,
        auth_validator: None,
        index_observer_factory: Some(Arc::clone(&index_observer_factory)),
        backfill_progress: Arc::new(dashmap::DashMap::new()),
        lock_registry: Some(lock_registry),
        topic_registry: Some(topic_registry),
        counter_registry: Some(counter_registry),
    };

    // Mirror NetworkModule::serve()'s scalar index rebuild for the topgun_server
    // path, which builds its router directly without going through NetworkModule.
    // Runs BEFORE set_ready() so queries arriving immediately after readiness see
    // populated indexes instead of empty-state false negatives. Reuses the
    // AppState backfill_progress Arc so progress entries written here are visible
    // to GET /api/admin/indexes/{map}/{attr}/status after set_ready. Must run
    // BEFORE `.with_state(state)` consumes the AppState.
    let scalar_path = std::path::PathBuf::from(
        std::env::var("TOPGUN_INDEX_PATH").unwrap_or_else(|_| "./scalar_indexes.json".to_string()),
    );
    let descriptors =
        topgun_server::network::handlers::admin::load_scalar_descriptors(&scalar_path);
    let scalar_count = descriptors.len();
    if scalar_count > 0 {
        let specs: Vec<topgun_server::service::domain::index::scalar_rebuild::ScalarRebuildSpec> =
            descriptors
                .into_iter()
                .map(
                    |d| topgun_server::service::domain::index::scalar_rebuild::ScalarRebuildSpec {
                        map_name: d.map_name,
                        attribute: d.attribute,
                        index_type: d.index_type,
                    },
                )
                .collect();
        topgun_server::service::domain::index::scalar_rebuild::rebuild_scalar_from_store(
            &index_observer_factory,
            &record_store_factory,
            &specs,
            &state.backfill_progress,
        )
        .await;
    }
    tracing::info!(
        target: "topgun_server::bootstrap",
        count = scalar_count,
        "scalar index restore complete"
    );

    // Build the axum router with state.
    // Build the router from the single source of truth shared with NetworkModule.
    // admin_routes() provides every production route; the only binary-specific
    // extra is mounting the WebSocket handler on / for browser clients (browsers
    // default to the root path when building the ws:// URL without a path).
    // All routes that admin_routes() already provides (/ws, /health, /sync,
    // /api/admin/*, /api/auth/*, /api/status) MUST NOT be re-declared here —
    // axum panics at startup on duplicate paths.
    let app = topgun_server::network::module::admin_routes(
        state.config.rate_limit_per_ip,
        state.config.rate_limit_burst,
    )
    // Browser WS dual-mount: /ws is already in admin_routes(); / is the
    // binary-only extra for clients that connect to the root path.
    .route(
        "/",
        get(topgun_server::network::handlers::ws_upgrade_handler),
    )
    .layer(topgun_server::network::middleware::build_http_layers(
        &state.config,
    ))
    .with_state(state);

    // Print port to stdout so the TS test harness can read it
    println!("PORT={bound_port}");

    // Mark the server as ready
    shutdown.set_ready();

    // Hand the controller a second handle so the graceful-shutdown closure
    // can flip /health to draining the moment SIGTERM lands, while the
    // outer scope keeps the original Arc for wait_for_drain below.
    let shutdown_for_drain = Arc::clone(&shutdown);

    // Serve until SIGTERM or SIGINT.
    // into_make_service_with_connect_info threads the peer SocketAddr into each
    // request so the governor's PeerIpKeyExtractor can identify the client IP.
    // Without it, every rate-limited admin route 500s with "Unable To Extract Key".
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        // Flip /health to 503 before the HTTP server starts closing so
        // any load balancer in front of us drains traffic away first.
        // Without this, /health stays "ready" until the process dies
        // and the LB happily forwards new requests into a dying server.
        shutdown_for_drain.trigger_shutdown();
        // Signal cluster services (HeartbeatService, EvictionOrchestrator, etc.)
        // to exit their loops. The write-behind flush loop is stopped separately
        // by hard_flush() below (via the store's own internal shutdown channel).
        let _ = shutdown_tx.send(true);
    })
    .await?;

    // After Hyper graceful shutdown returns, wait for any still-running
    // request handlers (tracked via ShutdownController::in_flight_guard) to
    // finish, with a bounded deadline so a stuck handler can't block exit
    // indefinitely. 30s matches typical k8s terminationGracePeriodSeconds.
    let drained = shutdown
        .wait_for_drain(std::time::Duration::from_secs(30))
        .await;
    if drained {
        tracing::info!("graceful shutdown complete");
    } else {
        tracing::warn!(
            "shutdown drain timed out after 30s — exiting with in-flight \
             requests still running. Investigate stuck handlers if persistent."
        );
    }

    // Drain the write-behind buffer so every write that was acked to a client
    // is durable before the process exits. The HTTP server is already drained
    // above, so no new client writes can arrive at this point. The drain is
    // bounded by shutdown_timeout_ms (default 30s); on timeout, still-pending
    // ops are logged via warn! and the process proceeds to exit.
    if let Err(err) = datastore.hard_flush().await {
        tracing::warn!(error = %err, "Write-behind drain encountered an error during shutdown");
    }

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
    hybrid_registry: Arc<HybridSearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
    /// Shared with `SearchService` so the observer can signal that an index needs
    /// population when writes were skipped due to no active subscriptions.
    needs_population: Arc<DashMap<String, AtomicBool>>,
    /// `SearchService` wired after construction via `init_search_service()`.
    /// Uses `OnceLock` to break the construction-order dependency: the factory
    /// is created before `RecordStoreFactory`, but `SearchService` requires
    /// `RecordStoreFactory`. The `OnceLock` is set immediately after `SearchService`
    /// is constructed and before the first map access triggers `create_observer`.
    search_service: std::sync::OnceLock<Arc<SearchService>>,
}

impl SearchObserverFactory {
    fn new(
        search_registry: Arc<SearchRegistry>,
        hybrid_registry: Arc<HybridSearchRegistry>,
        indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
        connection_registry: Arc<ConnectionRegistry>,
        needs_population: Arc<DashMap<String, AtomicBool>>,
    ) -> Self {
        Self {
            search_registry,
            hybrid_registry,
            indexes,
            connection_registry,
            needs_population,
            search_service: std::sync::OnceLock::new(),
        }
    }

    /// Wires the `SearchService` reference so `create_observer` can call
    /// `spawn_hybrid_notifier`. Must be called before the first map write.
    fn init_search_service(&self, svc: Arc<SearchService>) {
        let _ = self.search_service.set(svc);
    }
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
        let (observer, hybrid_rx) = SearchMutationObserver::new(
            map_name.to_string(),
            Arc::clone(&self.search_registry),
            Arc::clone(&self.hybrid_registry),
            Arc::clone(&self.indexes),
            Arc::clone(&self.connection_registry),
            config,
            Arc::clone(&self.needs_population),
        );
        // Spawn the hybrid notifier task if search_service is wired.
        // If not yet wired (should not happen in production), the hybrid_rx
        // is dropped and no hybrid deltas will be delivered for this map.
        if let Some(svc) = self.search_service.get() {
            svc.spawn_hybrid_notifier(hybrid_rx);
        } else {
            tracing::warn!(
                map = %map_name,
                "SearchObserverFactory: search_service not wired — hybrid_rx dropped, no hybrid deltas will be delivered"
            );
        }
        Some(Arc::new(observer))
    }
}

// ---------------------------------------------------------------------------
// Service wiring
// ---------------------------------------------------------------------------

/// Return type of [`build_services`]: the wired operation service, dispatcher,
/// connection registry, the three session-scoped registry Arcs, and the
/// `RecordStoreFactory` so the bootstrap can hand it to `EvictionOrchestrator`
/// without reaching into `build_services` internals.
type BuildServicesResult = (
    Arc<OperationService>,
    PartitionDispatcher,
    Arc<ConnectionRegistry>,
    Arc<LockRegistry>,
    Arc<TopicRegistry>,
    Arc<CounterRegistry>,
    Arc<RecordStoreFactory>,
    Arc<IndexObserverFactory>,
);

/// Cluster-mode parameters passed to [`build_services`] when starting with `--seed-nodes`.
///
/// Carries the shared `ClusterState` (used to satisfy `ClusterService` via
/// `ClusterStateAdapter`) and the `completion_registry` `DashMap` (shared between
/// `ClusterQueryCoordinator` and the cluster dispatch loop so `DagComplete`
/// messages from peer nodes can resolve the coordinator's awaiting receivers).
type ClusterParams = (
    Arc<ClusterState>,
    Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>>,
);

/// Wires all 7 domain services and builds the partition dispatcher.
///
/// Accepts `node_id`, `cluster_state`, and an optional `PolicyEvaluator` so
/// callers can wire RBAC enforcement into the Tower middleware pipeline.
/// When `policy_evaluator` is `Some`, every client operation is checked against
/// the policy store before reaching domain services.
///
/// Pass `cluster_params` as `Some(...)` in cluster mode so `QueryService`
/// routes queries through the distributed coordinator.  `None` preserves
/// single-node behaviour.
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
    datastore: Arc<dyn MapDataStore>,
    cluster_params: Option<ClusterParams>,
) -> BuildServicesResult {
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
    let hybrid_registry = Arc::new(HybridSearchRegistry::new());
    let search_indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let search_needs_population: Arc<DashMap<String, AtomicBool>> = Arc::new(DashMap::new());

    let search_observer_factory = Arc::new(SearchObserverFactory::new(
        Arc::clone(&search_registry),
        Arc::clone(&hybrid_registry),
        Arc::clone(&search_indexes),
        Arc::clone(&connection_registry),
        Arc::clone(&search_needs_population),
    ));
    let search_observer_factory_dyn: Arc<dyn ObserverFactory> =
        Arc::clone(&search_observer_factory) as Arc<dyn ObserverFactory>;

    // QueryRegistry shared between CrdtService (broadcast_query_updates) and QueryService.
    // QueryMutationObserver is no longer in the observer chain -- CrdtService handles
    // QUERY_UPDATE broadcast directly with writer exclusion and field projection.
    let query_registry = Arc::new(QueryRegistry::new());

    // MerkleSyncManager must be created before RecordStoreFactory so the
    // MerkleObserverFactory can be included in with_observer_factories().
    let merkle_manager = Arc::new(MerkleSyncManager::default());

    let merkle_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(MerkleObserverFactory::new(Arc::clone(&merkle_manager)));

    // Phase 1: construct the embedding observer factory before RecordStoreFactory exists.
    // Uses a noop provider for the test server (no real embedding service dependency).
    // The maps config is empty so no observer is registered for any map by default;
    // integration tests that need embedding can wire a custom VectorConfig.
    let embedding_vector_config = Arc::new(EmbeddingVectorConfig {
        provider: EmbeddingProviderConfig::Noop(NoopConfig { dimension: 4 }),
        maps: std::collections::HashMap::new(),
    });
    let embedding_provider: Arc<dyn EmbeddingProvider> =
        Arc::new(NoopEmbeddingProvider::new(&NoopConfig { dimension: 4 }));
    let embedding_factory = Arc::new(EmbeddingObserverFactory::new(
        EmbeddingConfig::default(),
        embedding_vector_config,
        embedding_provider,
    ));

    let index_observer_factory = Arc::new(IndexObserverFactory::new());

    #[allow(unused_mut)]
    let mut observer_factories: Vec<Arc<dyn ObserverFactory>> = vec![
        search_observer_factory_dyn,
        merkle_observer_factory,
        embedding_factory.clone(),
        Arc::clone(&index_observer_factory) as Arc<dyn ObserverFactory>,
    ];

    // When datafusion is enabled, register ArrowCacheObserverFactory so that
    // record mutations invalidate the Arrow cache for SQL query freshness.
    #[cfg(feature = "datafusion")]
    let _arrow_cache_manager = {
        let mgr = Arc::new(topgun_server::service::domain::arrow_cache::ArrowCacheManager::new());
        observer_factories.push(Arc::new(
            topgun_server::service::domain::arrow_cache::ArrowCacheObserverFactory::new(
                Arc::clone(&mgr),
            ),
        ));
        mgr
    };

    let record_store_factory = Arc::new(
        RecordStoreFactory::new(StorageConfig::default(), datastore, Vec::new())
            .with_observer_factories(observer_factories),
    );

    // Phase 2: inject the factory Arc and spawn the background embedding task.
    embedding_factory.init(Arc::clone(&record_store_factory));

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
    let query_merkle_manager =
        Arc::new(topgun_server::storage::query_merkle::QueryMerkleSyncManager::new());
    let query_svc_base = QueryService::new(
        Arc::clone(&query_registry),
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        Some(Arc::clone(&query_merkle_manager)),
        config.max_query_records,
        None,
        #[cfg(feature = "datafusion")]
        None,
    );
    // In cluster mode, construct a ClusterQueryCoordinator using the real
    // connection_registry and record_store_factory available here, and wire it
    // into QueryService.  Single-node startup passes None and leaves query_svc
    // using run_dag_local (unchanged behaviour).
    let query_svc = Arc::new(if let Some((cs, completion_registry)) = cluster_params {
        let cluster_svc: Arc<dyn ClusterService> = Arc::new(ClusterStateAdapter {
            state: cs,
            node_id: node_id.clone(),
        });
        let coordinator = Arc::new(ClusterQueryCoordinator::new(
            cluster_svc,
            Arc::clone(&connection_registry),
            Arc::clone(&record_store_factory),
            node_id.clone(),
            QueryConfig::default(),
            completion_registry,
        ));
        query_svc_base.with_coordinator(coordinator)
    } else {
        query_svc_base
    });
    let messaging_svc = Arc::new(MessagingService::new(Arc::clone(&connection_registry)));
    // Capture Arc<TopicRegistry> before messaging_svc is moved into the closure.
    let topic_registry_arc = messaging_svc.topic_registry_arc();
    let coordination_svc = Arc::new(CoordinationService::new(
        cluster_state,
        Arc::clone(&connection_registry),
    ));
    // Capture Arc<LockRegistry> before coordination_svc is moved into the closure.
    let lock_registry_arc = Arc::clone(coordination_svc.lock_registry());
    let search_svc = Arc::new(SearchService::new(
        search_registry,
        hybrid_registry,
        search_indexes,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        search_needs_population,
        Arc::clone(&index_observer_factory),
    ));
    // Wire the search_service back into the observer factory so hybrid notifier
    // tasks can be spawned when observers are created for each map.
    search_observer_factory.init_search_service(Arc::clone(&search_svc));
    // Two-phase OnceLock wiring: construct engine after search_svc, then set it.
    let hybrid_engine = topgun_server::service::domain::search::HybridSearchEngine::new(
        Arc::clone(&search_svc),
        Arc::clone(&record_store_factory),
        None,
    );
    search_svc.set_hybrid_engine(Arc::new(hybrid_engine));
    let persistence_svc = Arc::new(PersistenceService::new(
        Arc::clone(&connection_registry),
        node_id,
    ));
    // Capture Arc<CounterRegistry> before persistence_svc is moved into the closure.
    let counter_registry_arc = persistence_svc.counter_registry_arc();

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
        build_operation_pipeline(router, &config, evaluator_for_factory.clone())
    };

    let dispatch_config = DispatchConfig::default();
    let dispatcher = PartitionDispatcher::new(&dispatch_config, pipeline_factory);
    (
        classify_svc,
        dispatcher,
        connection_registry,
        lock_registry_arc,
        topic_registry_arc,
        counter_registry_arc,
        record_store_factory,
        Arc::clone(&index_observer_factory),
    )
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
