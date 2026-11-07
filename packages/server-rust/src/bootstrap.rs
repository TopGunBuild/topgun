//! Service wiring shared by every process that serves the full server: everything
//! between "the data store is open" and "the router is ready to serve".
//!
//! The server binary and the load harness both call [`build_services`],
//! [`start_background`] and [`into_router`], so a service, observer or background
//! task added here reaches both, and neither can drift from the other.
//!
//! The events of this module carry the target `topgun_server`, because that is the
//! target operators and log filters have seen for these lines since they were
//! emitted from the server binary. The target `topgun_server::bootstrap` that some
//! events carry (the binary's store-open lines, the tombstone epoch rebuild, the
//! scalar index restore and the tombstone-bytes lines of `storage::record`) is an
//! older log category that happens to share this module's path; neither target is
//! to be "corrected" to the other.

use std::sync::Arc;

use dashmap::DashMap;

use crate::cluster::messages::DagCompletePayload;
use crate::cluster::state::ClusterState;
use crate::network::config::NetworkConfig;
use crate::network::connection::ConnectionRegistry;
use crate::network::shutdown::ShutdownController;
use crate::service::domain::counter::CounterRegistry;
use crate::service::domain::index::IndexObserverFactory;
use crate::service::domain::journal::JournalStore;
use crate::service::domain::messaging::TopicRegistry;
use crate::service::domain::query::QueryRegistry;
use crate::service::domain::search::{HybridSearchRegistry, SearchRegistry};
use crate::service::domain::LockRegistry;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::expiry::SweepConfig;
use crate::service::middleware::ObservabilityHandle;
use crate::service::policy::{PolicyEvaluator, PolicyStore};
use crate::service::OperationService;
use crate::storage::eviction_config::EvictionConfig;
use crate::storage::factory::RecordStoreFactory;
use crate::storage::map_data_store::MapDataStore;
use crate::tombstone_frontier_impl::TombstoneFrontier;

/// Timeout of every operation the server runs, the expiry sweep's removals
/// included.
pub const OPERATION_TIMEOUT_MS: u64 = 5000;

/// Cluster-mode parameters passed to [`build_services`] when starting with `--seed-nodes`.
///
/// Carries the shared `ClusterState` (used to satisfy `ClusterService` via
/// `ClusterStateAdapter`) and the `completion_registry` `DashMap` (shared between
/// `ClusterQueryCoordinator` and the cluster dispatch loop so `DagComplete`
/// messages from peer nodes can resolve the coordinator's awaiting receivers).
pub type ClusterParams = (
    Arc<ClusterState>,
    Arc<DashMap<String, tokio::sync::oneshot::Sender<DagCompletePayload>>>,
);

/// What [`build_services`] wires: the operation service, the dispatcher, and every
/// registry and factory the router, the background tasks and disconnect cleanup
/// need.
pub struct Services {
    /// The `node_id` [`build_services`] was given; [`into_router`] puts it in the
    /// server config, so the two cannot differ.
    pub node_id: String,
    pub classify_svc: Arc<OperationService>,
    pub dispatcher: Arc<PartitionDispatcher>,
    pub connection_registry: Arc<ConnectionRegistry>,
    pub lock_registry: Arc<LockRegistry>,
    pub topic_registry: Arc<TopicRegistry>,
    pub counter_registry: Arc<CounterRegistry>,
    pub record_store_factory: Arc<RecordStoreFactory>,
    pub index_observer_factory: Arc<IndexObserverFactory>,
    pub query_registry: Arc<QueryRegistry>,
    pub search_registry: Arc<SearchRegistry>,
    pub hybrid_search_registry: Arc<HybridSearchRegistry>,
    pub journal_store: Arc<JournalStore>,
    pub frontier: Arc<TombstoneFrontier>,
}

/// The caller-specific inputs of [`into_router`].
pub struct AppParams {
    pub shutdown: Arc<ShutdownController>,
    pub network_config: NetworkConfig,
    pub jwt_secret: Option<String>,
    /// `None` in single-node mode.
    pub cluster_state: Option<Arc<ClusterState>>,
    pub observability: Option<Arc<ObservabilityHandle>>,
    pub policy_store: Arc<dyn PolicyStore>,
    pub admin_subjects: Arc<std::collections::HashSet<String>>,
    /// Both the route gate and `AppState.admin_enabled`.
    pub admin_enabled: bool,
    /// Vector index descriptors to rebuild from before serving; `None` rebuilds none.
    pub vector_index_path: Option<std::path::PathBuf>,
    /// Scalar index descriptors to rebuild from before serving.
    pub scalar_index_path: std::path::PathBuf,
}

/// What a caller uses after [`into_router`]: the router with its state applied and
/// the inputs of the connection reaper, which the caller spawns once it is ready.
pub struct Built {
    pub router: axum::Router,
    pub reaper_registry: Arc<ConnectionRegistry>,
    pub reaper_config: crate::network::ReaperConfig,
}

pub fn build_services(
    node_id: String,
    cluster_state: Arc<ClusterState>,
    policy_evaluator: Option<Arc<PolicyEvaluator>>,
    datastore: Arc<dyn MapDataStore>,
    cluster_params: Option<ClusterParams>,
) -> Result<Services, String> {
    let _ = (
        node_id,
        cluster_state,
        policy_evaluator,
        datastore,
        cluster_params,
    );
    unimplemented!()
}

pub async fn start_background(
    services: &Services,
    eviction: EvictionConfig,
    sweep: SweepConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let _ = (services, eviction, sweep, shutdown);
    unimplemented!()
}

pub async fn into_router(services: Services, params: AppParams) -> Built {
    let _ = (services, params);
    unimplemented!()
}
