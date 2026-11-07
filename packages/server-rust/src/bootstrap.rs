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

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use axum::routing::get;
use dashmap::DashMap;
use parking_lot::RwLock;
use topgun_core::{SystemClock, HLC};

use crate::cluster::messages::DagCompletePayload;
use crate::cluster::state::ClusterState;
use crate::cluster::types::{ClusterHealth, MembersView};
use crate::cluster::{ClusterChange, ClusterPartitionTable, ClusterService};
use crate::dag::types::QueryConfig;
use crate::dag::ClusterQueryCoordinator;
use crate::network::config::NetworkConfig;
use crate::network::connection::ConnectionRegistry;
use crate::network::handlers::AppState;
use crate::network::shutdown::ShutdownController;
use crate::service::config::ServerConfig;
use crate::service::dispatch::{DispatchConfig, PartitionDispatcher};
use crate::service::domain::coordination::CoordinationService;
use crate::service::domain::counter::CounterRegistry;
use crate::service::domain::crdt::CrdtService;
use crate::service::domain::embedding::{
    deterministic::DeterministicEmbeddingProvider, http::HttpEmbeddingProvider,
    noop::NoopEmbeddingProvider, ollama::OllamaEmbeddingProvider, DeterministicConfig,
    EmbeddingConfig, EmbeddingObserverFactory, EmbeddingProvider, EmbeddingProviderConfig,
    HttpProviderConfig, MapVectorConfig, NoopConfig, OllamaConfig,
    VectorConfig as EmbeddingVectorConfig,
};
use crate::service::domain::index::IndexObserverFactory;
use crate::service::domain::journal::JournalStore;
use crate::service::domain::messaging::{MessagingService, TopicRegistry};
use crate::service::domain::persistence::{PersistenceService, DEFAULT_JOURNAL_CAPACITY};
use crate::service::domain::query::{QueryRegistry, QueryService};
use crate::service::domain::schema::SchemaService;
use crate::service::domain::search::{
    HybridSearchRegistry, SearchConfig, SearchMutationObserver, SearchRegistry, SearchService,
    TantivyMapIndex,
};
use crate::service::domain::sync::SyncService;
use crate::service::domain::LockRegistry;
use crate::service::expiry::{spawn_expiry_sweep, ExpiryDispatch, SweepConfig};
use crate::service::middleware::{build_operation_pipeline, ObservabilityHandle};
use crate::service::operation::service_names;
use crate::service::policy::{PolicyEvaluator, PolicyStore};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::service::router::OperationRouter;
use crate::service::security::{SecurityConfig, WriteAdmission};
use crate::service::OperationService;
use crate::storage::durable_merkle::DurableMerkle;
use crate::storage::eviction_config::EvictionConfig;
use crate::storage::eviction_orchestrator::EvictionOrchestrator;
use crate::storage::factory::{ObserverFactory, RecordStoreFactory};
use crate::storage::impls::StorageConfig;
use crate::storage::map_data_store::MapDataStore;
use crate::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use crate::storage::mutation_observer::MutationObserver;
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
        // The shipped server runs these batch parameters, not SearchConfig::default()
        // (100 ms / 500); their cost on the write path is weighed separately.
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
                target: "topgun_server",
                map = %map_name,
                "SearchObserverFactory: search_service not wired — hybrid_rx dropped, no hybrid deltas will be delivered"
            );
        }
        Some(Arc::new(observer))
    }
}

/// Resolved server-side embedding / vector-search configuration, built from
/// environment variables. See [`build_embedding_setup`].
struct EmbeddingSetup {
    /// Provider config + per-map field config consumed by the write-path
    /// `EmbeddingObserverFactory`. `maps` is empty when semantic search is disabled.
    vector_config: Arc<EmbeddingVectorConfig>,
    /// Provider instance used by the write-path auto-embed observer. A harmless
    /// Noop placeholder when disabled (no maps means it is never invoked).
    write_provider: Arc<dyn EmbeddingProvider>,
    /// Provider wired into the query-path `HybridSearchEngine`. `None` when no
    /// provider is configured, so `semantic` queries return an explicit
    /// "no embedding provider configured" error instead of silent empty results.
    query_provider: Option<Arc<dyn EmbeddingProvider>>,
    /// `(map_name, dimension)` pairs to register an `_embedding` HNSW vector
    /// index for, so auto-embedded vectors are indexed and queryable.
    vector_indexes: Vec<(String, u16)>,
    /// Human-readable provider label for the startup config line ("none" when disabled).
    provider_label: String,
    /// Effective embedding dimension, when a provider is configured.
    dimension: Option<u16>,
}

/// Reads the embedding provider + vector-map configuration from the environment.
///
/// Recognized variables:
/// - `TOPGUN_EMBEDDING_PROVIDER`: `ollama` | `openai` (alias `http`) | `deterministic`
///   | `noop`. Unset / empty / `none` disables semantic search.
/// - `TOPGUN_EMBEDDING_MODEL`, `TOPGUN_EMBEDDING_BASE_URL`, `TOPGUN_EMBEDDING_API_KEY`,
///   `TOPGUN_EMBEDDING_DIMENSION`: provider connection + output dimension.
/// - `TOPGUN_VECTOR_MAPS`: JSON object mapping map name to `{ "fields": [...],
///   "dimension": N }`, declaring which maps/fields are auto-vectorized on write.
///
/// Honesty contract: when no provider is configured, the query path provider is
/// `None` (semantic → explicit error) and no maps are auto-embedded. `noop` is an
/// explicit, loudly-warned development mode — never a silent default.
///
/// On a malformed configuration (bad dimension, invalid JSON, dimension mismatch)
/// it returns the `FATAL:` line for the caller to print and exit on, rather than
/// serving a silently broken semantic surface.
#[allow(clippy::too_many_lines)]
fn build_embedding_setup() -> Result<EmbeddingSetup, String> {
    let disabled = || EmbeddingSetup {
        vector_config: Arc::new(EmbeddingVectorConfig {
            provider: EmbeddingProviderConfig::Noop(NoopConfig { dimension: 1 }),
            maps: std::collections::HashMap::new(),
        }),
        write_provider: Arc::new(NoopEmbeddingProvider::new(&NoopConfig { dimension: 1 })),
        query_provider: None,
        vector_indexes: Vec::new(),
        provider_label: "none".to_string(),
        dimension: None,
    };

    let kind = std::env::var("TOPGUN_EMBEDDING_PROVIDER")
        .unwrap_or_default()
        .trim()
        .to_lowercase();

    if kind.is_empty() || kind == "none" {
        if std::env::var("TOPGUN_VECTOR_MAPS").is_ok() {
            tracing::warn!(
                target: "topgun_server",
                "TOPGUN_VECTOR_MAPS is set but TOPGUN_EMBEDDING_PROVIDER is unset — \
                 no fields will be auto-embedded and semantic search stays disabled. \
                 Set TOPGUN_EMBEDDING_PROVIDER to enable vector/hybrid search."
            );
        }
        return Ok(disabled());
    }

    // Per-provider default dimension; overridable via TOPGUN_EMBEDDING_DIMENSION.
    let default_dim: Option<u16> = match kind.as_str() {
        "ollama" => Some(768),
        "deterministic" => Some(256),
        "noop" => Some(4),
        _ => None, // openai/http: model-specific, must be explicit
    };
    let dimension: u16 = match std::env::var("TOPGUN_EMBEDDING_DIMENSION") {
        Ok(raw) => raw
            .trim()
            .parse()
            .map_err(|_| format!("FATAL: TOPGUN_EMBEDDING_DIMENSION='{raw}' is not a valid u16"))?,
        Err(_) => default_dim.ok_or_else(|| {
            format!(
                "FATAL: TOPGUN_EMBEDDING_PROVIDER='{kind}' requires TOPGUN_EMBEDDING_DIMENSION \
                 (the embedding model's output size, e.g. 1536 for text-embedding-3-small)"
            )
        })?,
    };
    if dimension == 0 {
        // A zero dimension would panic the bucketing math and produce empty vectors.
        return Err("FATAL: TOPGUN_EMBEDDING_DIMENSION must be greater than 0".to_string());
    }

    let model = std::env::var("TOPGUN_EMBEDDING_MODEL").ok();
    let base_url = std::env::var("TOPGUN_EMBEDDING_BASE_URL").ok();
    let api_key = std::env::var("TOPGUN_EMBEDDING_API_KEY").ok();

    let (provider_config, provider): (EmbeddingProviderConfig, Arc<dyn EmbeddingProvider>) =
        match kind.as_str() {
            "ollama" => {
                let cfg = OllamaConfig {
                    base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
                    model: model.unwrap_or_else(|| "nomic-embed-text".to_string()),
                    dimension,
                };
                (
                    EmbeddingProviderConfig::Ollama(cfg.clone()),
                    Arc::new(OllamaEmbeddingProvider::new(cfg)),
                )
            }
            "openai" | "http" => {
                let base_url = base_url.ok_or_else(|| {
                    format!(
                        "FATAL: TOPGUN_EMBEDDING_PROVIDER='{kind}' requires TOPGUN_EMBEDDING_BASE_URL \
                         (e.g. https://api.openai.com)"
                    )
                })?;
                let model = model.ok_or_else(|| {
                    format!(
                        "FATAL: TOPGUN_EMBEDDING_PROVIDER='{kind}' requires TOPGUN_EMBEDDING_MODEL \
                         (e.g. text-embedding-3-small)"
                    )
                })?;
                let cfg = HttpProviderConfig {
                    base_url,
                    api_key,
                    model,
                    dimension,
                };
                (
                    EmbeddingProviderConfig::Http(cfg.clone()),
                    Arc::new(HttpEmbeddingProvider::new(cfg)),
                )
            }
            "deterministic" => (
                EmbeddingProviderConfig::Deterministic(DeterministicConfig { dimension }),
                Arc::new(DeterministicEmbeddingProvider::new(dimension)),
            ),
            "noop" => {
                tracing::warn!(
                    target: "topgun_server",
                    "TOPGUN_EMBEDDING_PROVIDER=noop — embeddings are ALL-ZERO vectors with no \
                     semantic meaning. Vector/hybrid 'semantic' results are NOT meaningful. \
                     This is a development-only mode; use 'ollama' or 'openai' in production."
                );
                (
                    EmbeddingProviderConfig::Noop(NoopConfig { dimension }),
                    Arc::new(NoopEmbeddingProvider::new(&NoopConfig { dimension })),
                )
            }
            other => {
                return Err(format!(
                    "FATAL: unknown TOPGUN_EMBEDDING_PROVIDER='{other}' \
                     (expected: ollama | openai | deterministic | noop)"
                ));
            }
        };

    // Parse per-map auto-embed config. Absent => provider is configured but no
    // maps are vectorized (semantic queries embed but find an empty index).
    let maps: std::collections::HashMap<String, MapVectorConfig> =
        if let Ok(raw) = std::env::var("TOPGUN_VECTOR_MAPS") {
            serde_json::from_str(&raw)
                .map_err(|e| format!("FATAL: TOPGUN_VECTOR_MAPS is not valid JSON: {e}"))?
        } else {
            tracing::warn!(
                target: "topgun_server",
                "TOPGUN_EMBEDDING_PROVIDER='{kind}' is set but TOPGUN_VECTOR_MAPS is empty — \
                 no maps will be auto-embedded. Set TOPGUN_VECTOR_MAPS to a JSON object like \
                 '{{\"docs\":{{\"fields\":[\"title\",\"body\"],\"dimension\":{dimension}}}}}'."
            );
            std::collections::HashMap::new()
        };

    let vector_config = EmbeddingVectorConfig {
        provider: provider_config,
        maps,
    };
    if let Err(e) = vector_config.validate() {
        return Err(format!(
            "FATAL: invalid TOPGUN_VECTOR_MAPS configuration: {e}"
        ));
    }

    let vector_indexes: Vec<(String, u16)> = vector_config
        .maps
        .iter()
        .map(|(map_name, cfg)| (map_name.clone(), cfg.dimension))
        .collect();

    Ok(EmbeddingSetup {
        vector_config: Arc::new(vector_config),
        write_provider: Arc::clone(&provider),
        query_provider: Some(provider),
        vector_indexes,
        provider_label: kind,
        dimension: Some(dimension),
    })
}

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
///
/// Synchronous, but it must run on a multi-thread tokio runtime: the Merkle seed
/// drives its async read through `block_in_place`, which panics elsewhere.
///
/// # Errors
///
/// The `FATAL:` line of a malformed embedding configuration, returned before
/// any task is spawned; the caller prints it and exits.
#[allow(clippy::too_many_lines)]
pub fn build_services(
    node_id: String,
    cluster_state: Arc<ClusterState>,
    policy_evaluator: Option<Arc<PolicyEvaluator>>,
    datastore: Arc<dyn MapDataStore>,
    cluster_params: Option<ClusterParams>,
) -> Result<Services, String> {
    let config = ServerConfig {
        node_id: node_id.clone(),
        default_operation_timeout_ms: OPERATION_TIMEOUT_MS,
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
    // Provider + per-map field config are resolved from the environment
    // (TOPGUN_EMBEDDING_PROVIDER / TOPGUN_VECTOR_MAPS). When no provider is
    // configured the maps config is empty, so no auto-embed observer is registered
    // and the query path stays honest (semantic → explicit "not configured" error).
    let embedding_setup = build_embedding_setup()?;
    let embedding_factory = Arc::new(EmbeddingObserverFactory::new(
        EmbeddingConfig::default(),
        Arc::clone(&embedding_setup.vector_config),
        Arc::clone(&embedding_setup.write_provider),
    ));

    let index_observer_factory = Arc::new(IndexObserverFactory::new());

    // Register an `_embedding` HNSW vector index for every auto-embedded map so the
    // vectors written back by the embedding hook are indexed and queryable. Both
    // the write path (IndexMutationObserver) and the query path (HybridSearchEngine)
    // read this same per-map IndexRegistry, so registering here wires both ends.
    for (map_name, dim) in &embedding_setup.vector_indexes {
        let registry = index_observer_factory.register_map(map_name.clone());
        registry.add_vector_index(
            "_embedding",
            *dim,
            topgun_core::vector::DistanceMetric::Cosine,
        );
    }

    // Single operator-facing line so the active embedding provider, dimension, and
    // auto-embedded maps can be confirmed from logs (mirrors the eviction/WAL line).
    tracing::info!(
        target: "topgun_server",
        embedding_provider = %embedding_setup.provider_label,
        embedding_dimension = ?embedding_setup.dimension,
        semantic_search_enabled = embedding_setup.query_provider.is_some(),
        vectorized_maps = ?embedding_setup
            .vector_indexes
            .iter()
            .map(|(m, _)| m.as_str())
            .collect::<Vec<_>>(),
        "embedding / vector-search configuration initialized"
    );

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
        let mgr = Arc::new(crate::service::domain::arrow_cache::ArrowCacheManager::new());
        observer_factories.push(Arc::new(
            crate::service::domain::arrow_cache::ArrowCacheObserverFactory::new(Arc::clone(&mgr)),
        ));
        mgr
    };

    // Keep a handle to the same durable backend the factory consumes so the
    // Merkle index can be seeded from persistence below. `datastore` is moved
    // into the factory on the next line, so the clone must be taken first.
    let datastore_for_merkle_seed: Arc<dyn MapDataStore> = Arc::clone(&datastore);

    let record_store_factory = Arc::new(
        RecordStoreFactory::new(StorageConfig::default(), datastore, Vec::new())
            .with_observer_factories(observer_factories),
    );

    // Warm up the in-memory Merkle tree as a non-authoritative accelerator.
    // The durable Merkle index (DurableMerkleIndex) is the authoritative source
    // for SYNC reads and does not depend on this warm-up. This accelerator may be
    // retired once the durable index is the sole read path.
    //
    // This is an index-only seed: `rebuild_from_datastore` reads keys + u32 leaf
    // hashes via `enumerate_leaves` and never loads record VALUES, so it is
    // RAM-ceiling-safe and does NOT rehydrate the in-memory engine — the
    // residency/eviction story is unchanged. The eager (pre-listener) variant is
    // chosen over a lazy-on-first-sync hook because it lives entirely in this
    // binary's startup path: the lazy path would have to reach into the
    // SyncService SYNC_INIT handler (a different file) and thread a data_store
    // into it. Eager seeding also makes the root trivially ready inside any
    // checkpoint quiesce window, since it completes before set_ready().
    //
    // `merkle_manager` here is the exact Arc moved into SyncService below, which
    // is the instance the real binary's SYNC_INIT path aggregates its root from.
    // `datastore_for_merkle_seed` is the same durable backend (write-behind-
    // wrapped redb) every other write goes through. Only primary
    // partitions are seeded (rebuild_from_datastore uses is_backup=false), which
    // matches the SYNC_INIT root path (handle_sync_init aggregates the primary
    // LWW / OR-Map roots); backup partitions are intentionally not on this path.
    //
    // `build_services` is synchronous but always runs inside the multi-threaded
    // `#[tokio::main]` runtime, so the async seed is driven via the
    // `block_in_place` + `Handle::block_on` bridge (the same pattern the sim
    // harness uses to run async code from sync contexts). This blocks the
    // current bootstrap task — acceptable because the seed runs once, before the
    // listener accepts connections, and reads only keys+hashes.
    if !datastore_for_merkle_seed.is_null() {
        let seed_manager = Arc::clone(&merkle_manager);
        let seed_store = Arc::clone(&datastore_for_merkle_seed);
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                match seed_store.list_maps().await {
                    Ok(maps) => {
                        let mut seeded = 0usize;
                        let total = maps.len();
                        for map_name in maps.into_iter().filter(|m| !m.starts_with("$sys/")) {
                            if let Err(err) = seed_manager
                                .rebuild_from_datastore(&map_name, &seed_store)
                                .await
                            {
                                // A single map's rebuild failure must not abort
                                // startup: log and continue so the remaining maps
                                // still answer sync correctly. The unseeded map
                                // degrades to an empty tree (pre-existing
                                // behavior), never to wrong leaves.
                                tracing::warn!(
                                    target: "topgun_server",
                                    map = %map_name,
                                    error = %err,
                                    "Merkle index seed: rebuild failed for map; \
                                     leaving its trees empty (will reseed on next write)"
                                );
                            } else {
                                seeded += 1;
                            }
                        }
                        tracing::info!(
                            target: "topgun_server",
                            durable_maps = total,
                            seeded,
                            "Merkle index seeded from datastore (keys+hashes only, no value load)"
                        );
                    }
                    Err(err) => {
                        // Enumeration failure is non-fatal: the trees stay empty
                        // and repopulate lazily as writes arrive. Surfacing it
                        // lets an operator distinguish "no durable maps" from
                        // "could not read the catalog".
                        tracing::warn!(
                            target: "topgun_server",
                            error = %err,
                            "Merkle index seed: durable map enumeration failed; \
                             trees will populate from live writes"
                        );
                    }
                }
            });
        });
    }

    // One per-key writer for every writer of this node's record stores. Built
    // before the embedding task starts so that task is handed the same
    // registry as the CRDT, SYNC and prune paths below.
    let key_writer = Arc::new(crate::service::domain::key_writer::KeyWriterRegistry::new());

    // Phase 2: inject the factory Arc and spawn the background embedding task.
    embedding_factory.init(Arc::clone(&record_store_factory), Arc::clone(&key_writer));

    // The clock client writes are stamped with. The SYNC service stamps pushed
    // OR entries from the SAME `Arc`: a second clock with this node id could
    // mint one timestamp for two writes (TG-TTL-001).
    let wv_hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
        config.node_id.clone(),
        Box::new(SystemClock),
    )));
    let write_validator = Arc::new(WriteAdmission::new(
        Arc::new(SecurityConfig::default()),
        Arc::clone(&wv_hlc),
    ));

    // Event Journal: one shared ring buffer, written by the CRDT mutation path
    // and read/subscribed by the persistence service. Enabled by default so
    // getEventJournal/useEventJournal work out of the box; an operator can shed
    // its per-write cost with TOPGUN_JOURNAL_ENABLED=false (reads then observe an
    // empty buffer — an explicit, startup-logged opt-out).
    let journal_enabled = std::env::var("TOPGUN_JOURNAL_ENABLED")
        .map(|v| {
            !matches!(
                v.trim().to_lowercase().as_str(),
                "false" | "0" | "no" | "off"
            )
        })
        .unwrap_or(true);
    let journal_capacity = std::env::var("TOPGUN_JOURNAL_CAPACITY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|c| *c > 0)
        .unwrap_or(DEFAULT_JOURNAL_CAPACITY);
    // The journal holds its readers to the same policies as the pipeline, so a
    // read or subscription without a map sees only the maps its principal may read.
    let mut journal_store = JournalStore::with_enabled(journal_capacity, journal_enabled);
    if let Some(evaluator) = &policy_evaluator {
        journal_store = journal_store.with_policy(Arc::clone(evaluator));
    }
    let journal_store = Arc::new(journal_store);
    tracing::info!(
        target: "topgun_server",
        journal_enabled,
        journal_capacity,
        "event journal initialized"
    );

    // Shared causal frontier + per-key writer, constructed ONCE here so the same
    // instances flow into CrdtService (epoch stamping + OR-write prune),
    // SyncService (covering-epoch conveyance + SYNC-leaf prune), and AppState (the
    // durable per-device cursor store). A second frontier would fork the epoch
    // authority — stamping epochs no client ever ACKs — and two per-key writers
    // over one store would let a prune and an OR write race.
    let frontier = Arc::new(crate::tombstone_frontier_impl::TombstoneFrontier::new(
        Some(record_store_factory.data_store()),
    ));
    // Operator-tunable epoch width (stamped tombstone ops per epoch), surfaced in
    // the config log so the active value is confirmable without reading source.
    // An invalid/zero value falls back to the default LOUDLY — a silent fallback
    // would leave the operator believing their setting took effect.
    let epoch_width = match std::env::var("TOPGUN_EPOCH_WIDTH") {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(w) if w > 0 => w,
            _ => {
                tracing::warn!(
                    target: "topgun_server",
                    rejected_value = %raw,
                    default = crate::tombstone_frontier_impl::DEFAULT_EPOCH_WIDTH,
                    "invalid TOPGUN_EPOCH_WIDTH (expected a positive integer); using the default"
                );
                crate::tombstone_frontier_impl::DEFAULT_EPOCH_WIDTH
            }
        },
        Err(_) => crate::tombstone_frontier_impl::DEFAULT_EPOCH_WIDTH,
    };
    frontier.set_epoch_width(epoch_width);
    tracing::info!(
        target: "topgun_server",
        epoch_width = frontier.epoch_width(),
        "tombstone epoch counter initialized (the epoch prune runs on the background task)"
    );

    // Arc-wrap all domain services so they can be shared across N+1 worker pipelines.
    let crdt_svc = Arc::new(
        CrdtService::new(
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
            write_validator,
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        )
        .with_journal(Arc::clone(&journal_store))
        .with_frontier(Arc::clone(&frontier))
        .with_key_writer(Arc::clone(&key_writer)),
    );
    // Wire the durable Merkle index so SYNC tree-walk reads resolve from the
    // persisted backend, not the eviction-coupled in-memory tree.
    // `datastore_for_merkle_seed` is the same write-behind-wrapped durable
    // backend the write path persists to, so the enumerated leaf set is
    // authoritative over records that are durable but not resident.
    // `build_services` always runs on the multi-threaded runtime, so
    // `build_session`'s internal `block_in_place` is safe.
    //
    // Only wire it for durable backends: the Null backend persists nothing, so a
    // durable session would always be empty — the in-memory MerkleSyncManager is
    // the only valid SYNC source there. This mirrors the merkle-seed guard below.
    let sync_base = SyncService::new(
        merkle_manager,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        Arc::clone(&query_registry),
    )
    // Same frontier + per-key writer as CrdtService: covering-epoch conveyance
    // reads the epoch counter CrdtService stamps, and the SYNC-leaf prune
    // serializes against OR writes on the shared writer.
    .with_frontier(Arc::clone(&frontier), Arc::clone(&key_writer))
    .with_hlc(wv_hlc);
    let sync_svc = Arc::new(if datastore_for_merkle_seed.is_null() {
        sync_base
    } else {
        sync_base.with_durable_index(
            Arc::new(DurableMerkle),
            Arc::clone(&datastore_for_merkle_seed),
        )
    });
    // The ONE long-lived prune task, spawned here so exactly one exists per
    // process in BOTH the cluster and the single-node branch that call this fn.
    // Its trigger sites only leave a wake permit; without this consumer the
    // server would reclaim no tombstone at all while every write path kept
    // returning successfully.
    // The handle is deliberately detached: the task is meant to outlive this fn and end only
    // at runtime teardown. A `None` is not: it is the one thing this call can silently do
    // nothing about, so it is reported rather than discarded.
    if crate::service::domain::crdt::spawn_prune_task(
        Arc::clone(&frontier),
        Arc::clone(&record_store_factory),
        Arc::clone(&key_writer),
    )
    .is_none()
    {
        tracing::warn!(
            target: "topgun_server",
            "no prune task was spawned here: one is already claimed for this frontier. That \
             claim is the only reason this call declines, and the task holding it is the one \
             that reclaims tombstones"
        );
    }
    let query_svc_base = QueryService::new(
        Arc::clone(&query_registry),
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        config.max_query_records,
        Some(Arc::clone(&index_observer_factory)),
        #[cfg(feature = "datafusion")]
        None,
    )
    .with_hlc(Arc::clone(&hlc));
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
    // Capture Arc<QueryRegistry> before query_svc is moved into the closure.
    let query_registry_arc = query_svc.query_registry_arc();
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
    // Capture the search subscription registries before search_svc is moved into
    // the closure, so disconnect cleanup can prune live search subscriptions.
    let search_registry_arc = search_svc.search_registry_arc();
    let hybrid_search_registry_arc = search_svc.hybrid_search_registry_arc();
    // Wire the search_service back into the observer factory so hybrid notifier
    // tasks can be spawned when observers are created for each map.
    search_observer_factory.init_search_service(Arc::clone(&search_svc));
    // Two-phase OnceLock wiring: construct engine after search_svc, then set it.
    // The query-path embedding provider (if configured) lets `semantic` queries
    // auto-embed their query text; `None` makes them return an explicit
    // "no embedding provider configured" error rather than silent empty results.
    let hybrid_engine = crate::service::domain::search::HybridSearchEngine::new(
        Arc::clone(&search_svc),
        Arc::clone(&record_store_factory),
        embedding_setup.query_provider.clone(),
    );
    search_svc.set_hybrid_engine(Arc::new(hybrid_engine));
    let persistence_svc = Arc::new(PersistenceService::with_journal_store(
        Arc::clone(&connection_registry),
        node_id.clone(),
        Arc::clone(&journal_store),
    ));
    // Capture Arc<CounterRegistry> before persistence_svc is moved into the closure.
    let counter_registry_arc = persistence_svc.counter_registry_arc();
    // The CRDT write path and persistence read/subscribe path share this store;
    // reuse it for disconnect cleanup (pruning per-connection journal subscriptions).
    let journal_store_arc = Arc::clone(&journal_store);

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
    Ok(Services {
        node_id,
        classify_svc,
        dispatcher: Arc::new(dispatcher),
        connection_registry,
        lock_registry: lock_registry_arc,
        topic_registry: topic_registry_arc,
        counter_registry: counter_registry_arc,
        record_store_factory,
        index_observer_factory: Arc::clone(&index_observer_factory),
        query_registry: query_registry_arc,
        search_registry: search_registry_arc,
        hybrid_search_registry: hybrid_search_registry_arc,
        journal_store: journal_store_arc,
        frontier,
    })
}

/// Starts what must run before the router serves: the tombstone epoch rebuild,
/// then the eviction orchestrator and the expiry sweep. Returns the sweep's
/// handle, which the caller waits on (bounded) at stop.
///
/// # Errors
///
/// The `FATAL:` line of a failed tombstone epoch rebuild, returned before
/// anything is spawned; the caller prints it and exits.
pub async fn start_background(
    services: &Services,
    eviction: EvictionConfig,
    sweep: SweepConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<tokio::task::JoinHandle<()>, String> {
    // Unclean-recovery rebuild of the tombstone epoch index.
    // The RAM epoch index is never persisted on the hot path, so after WAL recovery
    // it is empty: re-stamp every live tombstone into ONE fresh maximally-lagging
    // recovery epoch and bump the epoch counter past every persisted client cursor,
    // so nothing is prune-eligible — and no stale high cursor prematurely licenses a
    // prune of a freshly-numbered epoch — until every tracked client re-confirms.
    //
    // This MUST run in the pre-listener window: strictly BEFORE the caller's
    // set_ready()/serve(), so `durable_epoch_watermark` stays 0 (prune dark, gate
    // transparent) until the rebuild completes and a reconnecting client never observes a
    // half-rebuilt index. It runs on the live serving path (the router
    // `into_router` builds — not a secondary router, so the dual-router trap does
    // not apply). A scan failure is FATAL: continuing with an
    // empty index and a un-bumped counter would let a rehydrated stale-high cursor
    // prune a new epoch → resurrection. Fail closed, matching WAL-recovery.
    match services.frontier.rebuild_from_durable_store().await {
        Ok(recovery_epoch) => tracing::info!(
            target: "topgun_server::bootstrap",
            recovery_epoch,
            "tombstone epoch index rebuilt (pre-listener); prune fenced maximally-lagging until clients re-confirm"
        ),
        Err(err) => {
            return Err(format!(
                "FATAL: tombstone epoch index rebuild failed: {err}"
            ))
        }
    }

    // Spawn the eviction orchestrator after services are wired so it observes
    // every store the factory will create. The orchestrator terminates within
    // one `interval_ms` of the caller's shutdown signal via its clone of
    // `shutdown`.
    let orchestrator = EvictionOrchestrator::new(
        eviction,
        Arc::clone(&services.record_store_factory),
        shutdown.clone(),
    );
    tokio::spawn(orchestrator.run());

    // Through the dispatcher, so each removal is routed, ordered and
    // back-pressured like any other operation (TG-TTL-001).
    Ok(spawn_expiry_sweep(
        Arc::clone(&services.record_store_factory),
        Arc::clone(&services.dispatcher) as Arc<dyn ExpiryDispatch>,
        sweep,
        shutdown,
    ))
}

/// Builds the server's state and router, and rebuilds the vector and scalar
/// indexes from the stored records before it is served.
pub async fn into_router(services: Services, params: AppParams) -> Built {
    let state = AppState {
        registry: services.connection_registry,
        shutdown: params.shutdown,
        config: Arc::new(params.network_config),
        start_time: Instant::now(),
        observability: params.observability,
        operation_service: Some(services.classify_svc),
        dispatcher: Some(services.dispatcher),
        jwt_secret: params.jwt_secret,
        cluster_state: params.cluster_state,
        store_factory: Some(Arc::clone(&services.record_store_factory)),
        // The SAME causal frontier instance CrdtService/SyncService received in
        // build_services — a single epoch-counter + low-water-mark authority, NOT a
        // second frontier.
        frontier: Some(services.frontier),
        server_config: Some(Arc::new(ArcSwap::from_pointee(ServerConfig {
            node_id: services.node_id,
            ..ServerConfig::default()
        }))),
        policy_store: Some(params.policy_store),
        admin_subjects: params.admin_subjects,
        auth_providers: Arc::new(vec![]),
        refresh_grant_store: None,
        auth_validator: None,
        index_observer_factory: Some(Arc::clone(&services.index_observer_factory)),
        backfill_progress: Arc::new(dashmap::DashMap::new()),
        lock_registry: Some(services.lock_registry),
        topic_registry: Some(services.topic_registry),
        counter_registry: Some(services.counter_registry),
        // Disconnect-cleanup registries: dropping a connection must prune the
        // subscriptions it held in each of these so the server does not leak
        // per-connection subscription state for the lifetime of the process.
        query_registry: Some(services.query_registry),
        journal_store: Some(services.journal_store),
        search_registry: Some(services.search_registry),
        hybrid_search_registry: Some(services.hybrid_search_registry),
        // One decision, two enforcement points: the route gate below and the
        // extractor guard (`AppState.admin_enabled`) read the SAME boolean, so they
        // can never drift.
        admin_enabled: params.admin_enabled,
        // The caller resolves the vector-index sidecar path at the construction
        // boundary, so request handlers stay env-free.
        vector_index_path: params.vector_index_path,
    };

    // Rebuild every vector index from the stored records before set_ready(),
    // so VECTOR_SEARCH is not served against empty indexes after a restart.
    // build_services has registered the `_embedding` indexes of
    // TOPGUN_VECTOR_MAPS by now, so they are filled too (TG-VEC-001).
    if let Some(vector_index_path) = &state.vector_index_path {
        crate::service::domain::index::vector_index::rebuild_from_store(
            &services.index_observer_factory,
            &services.record_store_factory,
            &crate::network::handlers::admin::load_vector_descriptors(vector_index_path),
            &state.backfill_progress,
        )
        .await;
    }

    // Mirror NetworkModule::serve()'s scalar index rebuild for the topgun_server
    // path, which builds its router directly without going through NetworkModule.
    // Runs BEFORE set_ready() so queries arriving immediately after readiness see
    // populated indexes instead of empty-state false negatives. Reuses the
    // AppState backfill_progress Arc so progress entries written here are visible
    // to GET /api/admin/indexes/{map}/{attr}/status after set_ready. Must run
    // BEFORE `.with_state(state)` consumes the AppState.
    let descriptors =
        crate::network::handlers::admin::load_scalar_descriptors(&params.scalar_index_path);
    let scalar_count = descriptors.len();
    if scalar_count > 0 {
        let specs: Vec<crate::service::domain::index::scalar_rebuild::ScalarRebuildSpec> =
            descriptors
                .into_iter()
                .map(
                    |d| crate::service::domain::index::scalar_rebuild::ScalarRebuildSpec {
                        map_name: d.map_name,
                        attribute: d.attribute,
                        index_type: d.index_type,
                    },
                )
                .collect();
        crate::service::domain::index::scalar_rebuild::rebuild_scalar_from_store(
            &services.index_observer_factory,
            &services.record_store_factory,
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
    // Capture reaper inputs before `state` is moved into `.with_state()`.
    // This router is hand-built (it does NOT go through NetworkModule::serve),
    // so its caller must spawn the connection reaper too —
    // otherwise idle/half-open and never-authed connections leak in the real
    // server while only the NetworkModule path is protected.
    let reaper_registry = Arc::clone(&state.registry);
    let reaper_config = crate::network::ReaperConfig::from(&state.config.connection);

    let app = crate::network::module::admin_routes(
        state.config.rate_limit_per_ip,
        state.config.rate_limit_burst,
        params.admin_enabled,
        state.config.trust_forwarded_for,
    )
    // Browser WS dual-mount: /ws is already in admin_routes(); / is the
    // binary-only extra for clients that connect to the root path.
    .route("/", get(crate::network::handlers::ws_upgrade_handler))
    .layer(crate::network::middleware::build_http_layers(&state.config))
    .with_state(state);

    Built {
        router: app,
        reaper_registry,
        reaper_config,
    }
}
