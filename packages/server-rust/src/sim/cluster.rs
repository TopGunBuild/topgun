//! Simulation cluster harness for deterministic testing.
//!
//! Provides [`SimCluster`] (N-node orchestrator) and [`SimNode`] (single node
//! with full service stack backed by in-memory storage). All types are behind
//! `#[cfg(feature = "simulation")]`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tower::Service;

use topgun_core::messages::base::Query;
use topgun_core::messages::query::QueryResultEntry;
use topgun_core::messages::sync::{ClientOpMessage, OpBatchMessage, OpBatchPayload};
use topgun_core::{ClientOp, LWWRecord, ORMapRecord, SystemClock, Timestamp, HLC};

use async_trait::async_trait;

use crate::cluster::dispatch::{run_cluster_dispatch_loop, ClusterDispatchContext};
use crate::cluster::messages::DagCompletePayload;
use crate::cluster::state::{ClusterChange, ClusterChannels, ClusterPartitionTable, ClusterState};
use crate::cluster::traits::ClusterService;
use crate::cluster::types::{ClusterConfig, ClusterHealth, MembersView};
use crate::dag::coordinator::ClusterQueryCoordinator;
use crate::network::connection::ConnectionRegistry;
use crate::service::domain::query::QueryRegistry;
use crate::service::domain::search::{HybridSearchRegistry, SearchRegistry};
use crate::service::domain::{
    CoordinationService, CrdtService, MessagingService, PersistenceService, QueryService,
    SchemaService, SearchService, SyncService,
};
use crate::service::operation::{service_names, CallerOrigin, Operation, OperationContext};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::service::router::OperationRouter;
use crate::service::security::{SecurityConfig, WriteAdmission};
use crate::storage::datastores::NullDataStore;
use crate::storage::factory::{ObserverFactory, RecordStoreFactory};
use crate::storage::impls::StorageConfig;
use crate::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use crate::storage::record::{OrMapEntry, RecordValue};

use super::network::{SimNetwork, SimTransport};

// ---------------------------------------------------------------------------
// SimClusterService — thin wrapper around ClusterState for dyn ClusterService
// ---------------------------------------------------------------------------

/// Minimal `ClusterService` implementation for simulation nodes.
///
/// Wraps a shared `ClusterState` so that `ClusterQueryCoordinator` can
/// query membership and partition tables during distributed GROUP BY execution.
struct SimClusterService {
    state: Arc<ClusterState>,
    node_id: String,
}

#[async_trait]
impl ManagedService for SimClusterService {
    fn name(&self) -> &'static str {
        "sim-cluster"
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

#[async_trait]
impl ClusterService for SimClusterService {
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
        tokio::sync::mpsc::unbounded_channel().1
    }
    fn health(&self) -> ClusterHealth {
        let view = self.state.current_view();
        ClusterHealth {
            node_count: view.members.len(),
            active_nodes: view.members.len(),
            suspect_nodes: 0,
            partition_table_version: 1,
            active_migrations: 0,
            is_master: self.state.is_master(),
            master_node_id: self.master_id(),
        }
    }
}

// ---------------------------------------------------------------------------
// SimNode
// ---------------------------------------------------------------------------

/// A single simulated `TopGun` server node with a full service stack.
///
/// Uses in-memory storage (`NullDataStore` + `HashMapStorage` via
/// `RecordStoreFactory`) instead of `PostgreSQL`. The node's services are
/// invoked directly through the Tower `Service<Operation>` trait rather
/// than through `WebSocket` connections.
pub struct SimNode {
    /// Unique identifier for this node.
    pub node_id: String,
    /// `CrdtService` handle for direct invocation via Tower `Service` trait.
    pub crdt_service: Arc<CrdtService>,
    /// `RecordStoreFactory` for reading records from in-memory storage.
    pub record_store_factory: Arc<RecordStoreFactory>,
    /// Full operation router with all 7 domain services registered.
    pub operation_router: OperationRouter,
    /// Cluster state for this node.
    pub cluster_state: Arc<ClusterState>,
    /// Shared transport for inter-node message delivery.
    pub transport: SimTransport,
    /// Connection registry for this node.
    pub connection_registry: Arc<ConnectionRegistry>,
    /// Sender half of the inbound cluster message channel. Used by the sim
    /// transport to inject messages into the dispatch loop via
    /// `handle_cluster_peer_frame`.
    pub inbound_tx: mpsc::Sender<crate::cluster::state::InboundClusterMessage>,
    /// Shared completion registry for `DagComplete` responses. The same `Arc`
    /// is shared between the dispatch loop and the `ClusterQueryCoordinator`.
    pub completion_registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>>,
    /// Coordinator for distributed GROUP BY queries.
    pub coordinator: Arc<ClusterQueryCoordinator>,
    /// Shared Event Journal store, written by the CRDT path and read/subscribed
    /// by the persistence service. Exposed so sim tests can read appended events.
    pub journal_store: Arc<crate::service::domain::journal::JournalStore>,
    /// Handle for the dispatch loop background task. Aborted on `kill()`.
    dispatch_handle: tokio::task::JoinHandle<()>,
    /// Whether this node is currently alive.
    alive: bool,
}

impl SimNode {
    /// Builds a fully initialized `SimNode` with all 7 domain services wired.
    ///
    /// Follows the same wiring pattern as the `setup()` helper in `lib.rs`
    /// integration tests, using `NullDataStore` for persistence and
    /// `HashMapStorage` (via `RecordStoreFactory`) for in-memory storage.
    ///
    /// # Errors
    ///
    /// Returns an error if service wiring fails (should not happen with
    /// in-memory storage).
    #[allow(clippy::too_many_lines)]
    pub fn build(
        node_id: impl Into<String>,
        _seed: u64,
        transport: SimTransport,
    ) -> anyhow::Result<Self> {
        let node_id = node_id.into();

        let hlc = Arc::new(Mutex::new(HLC::new(node_id.clone(), Box::new(SystemClock))));
        let write_validator = Arc::new(WriteAdmission::new(
            Arc::new(SecurityConfig::default()),
            hlc,
        ));

        let cluster_config = Arc::new(ClusterConfig::default());
        let (cluster_state, _rx) = ClusterState::new(cluster_config, node_id.clone());
        let cluster_state = Arc::new(cluster_state);
        let connection_registry = Arc::new(ConnectionRegistry::new());

        // Cluster dispatch loop: channels + shared completion registry.
        let (channels, receivers) = ClusterChannels::new(256);
        let inbound_tx = channels.inbound_messages;
        let completion_registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>> =
            Arc::new(DashMap::new());

        // MerkleSyncManager and observer factory for Merkle tree tracking.
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        let merkle_observer_factory: Arc<dyn ObserverFactory> =
            Arc::new(MerkleObserverFactory::new(Arc::clone(&merkle_manager)));

        let record_store_factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_observer_factories(vec![merkle_observer_factory]),
        );

        let query_registry = Arc::new(QueryRegistry::new());

        // Shared Event Journal store: the CRDT write path appends to it, the
        // persistence service reads/subscribes from it. Same Arc handed to both.
        let journal_store = Arc::new(crate::service::domain::journal::JournalStore::new(10_000));

        // Build all 7 domain services.
        let crdt_service = Arc::new(
            CrdtService::new(
                Arc::clone(&record_store_factory),
                Arc::clone(&connection_registry),
                write_validator,
                Arc::clone(&query_registry),
                Arc::new(SchemaService::new()),
            )
            .with_journal(Arc::clone(&journal_store)),
        );

        let mut router = OperationRouter::new();

        router.register(service_names::CRDT, Arc::clone(&crdt_service));

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
                query_registry,
                Arc::clone(&record_store_factory),
                Arc::clone(&connection_registry),
                None,
                10_000,
                None,
                #[cfg(feature = "datafusion")]
                None,
            )),
        );

        router.register(
            service_names::MESSAGING,
            Arc::new(MessagingService::new(Arc::clone(&connection_registry))),
        );

        router.register(
            service_names::COORDINATION,
            Arc::new(CoordinationService::new(
                Arc::clone(&cluster_state),
                Arc::clone(&connection_registry),
            )),
        );

        let search_needs_population = Arc::new(dashmap::DashMap::new());
        let index_observer_factory =
            Arc::new(crate::service::domain::index::IndexObserverFactory::new());
        let search_svc = Arc::new(SearchService::new(
            Arc::new(SearchRegistry::new()),
            Arc::new(HybridSearchRegistry::new()),
            Arc::new(parking_lot::RwLock::new(HashMap::new())),
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
            search_needs_population,
            index_observer_factory,
        ));
        let hybrid_engine = crate::service::domain::search::HybridSearchEngine::new(
            Arc::clone(&search_svc),
            Arc::clone(&record_store_factory),
            None,
        );
        search_svc.set_hybrid_engine(Arc::new(hybrid_engine));
        router.register(service_names::SEARCH, search_svc);

        router.register(
            service_names::PERSISTENCE,
            Arc::new(PersistenceService::with_journal_store(
                Arc::clone(&connection_registry),
                node_id.clone(),
                Arc::clone(&journal_store),
            )),
        );

        // Spawn the cluster dispatch loop in a background task.
        let dispatch_ctx = ClusterDispatchContext {
            local_node_id: node_id.clone(),
            completion_registry: Arc::clone(&completion_registry),
            record_store_factory: Arc::clone(&record_store_factory),
            connection_registry: Arc::clone(&connection_registry),
        };
        let dispatch_handle = tokio::spawn(run_cluster_dispatch_loop(
            dispatch_ctx,
            receivers.inbound_messages,
        ));

        // Build the ClusterQueryCoordinator with the shared completion_registry
        // so the dispatch loop can resolve oneshot receivers created by the coordinator.
        let sim_cluster_service: Arc<dyn ClusterService> = Arc::new(SimClusterService {
            state: Arc::clone(&cluster_state),
            node_id: node_id.clone(),
        });
        let coordinator = Arc::new(ClusterQueryCoordinator::new(
            sim_cluster_service,
            Arc::clone(&connection_registry),
            Arc::clone(&record_store_factory),
            node_id.clone(),
            crate::dag::types::QueryConfig::default(),
            Arc::clone(&completion_registry),
        ));

        Ok(SimNode {
            node_id,
            crdt_service,
            record_store_factory,
            operation_router: router,
            cluster_state,
            transport,
            connection_registry,
            inbound_tx,
            completion_registry,
            coordinator,
            journal_store,
            dispatch_handle,
            alive: true,
        })
    }

    /// Returns whether this node is currently alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Marks this node as dead and aborts the dispatch loop (simulates crash).
    pub fn kill(&mut self) {
        self.alive = false;
        self.dispatch_handle.abort();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Converts a `topgun_core::Value` into an `rmpv::Value` for wire-format ops.
///
/// Direct structural conversion to avoid the round-trip through `rmp_serde`
/// which would serialize the enum as a tagged map `{"String": "..."}` rather
/// than a plain `rmpv::Value::String`. The symmetry with `rmpv_to_value` in
/// `crdt.rs` ensures values survive a store → wire → store round-trip unchanged.
fn value_to_rmpv(v: &topgun_core::Value) -> rmpv::Value {
    match v {
        topgun_core::Value::Null => rmpv::Value::Nil,
        topgun_core::Value::Bool(b) => rmpv::Value::Boolean(*b),
        topgun_core::Value::Int(n) => rmpv::Value::Integer((*n).into()),
        topgun_core::Value::Float(f) => rmpv::Value::F64(*f),
        topgun_core::Value::String(s) => rmpv::Value::String(s.as_str().into()),
        topgun_core::Value::Bytes(b) => rmpv::Value::Binary(b.clone()),
        topgun_core::Value::Array(arr) => {
            rmpv::Value::Array(arr.iter().map(value_to_rmpv).collect())
        }
        topgun_core::Value::Map(map) => {
            let entries: Vec<(rmpv::Value, rmpv::Value)> = map
                .iter()
                .map(|(k, val)| (rmpv::Value::String(k.as_str().into()), value_to_rmpv(val)))
                .collect();
            rmpv::Value::Map(entries)
        }
    }
}

/// Builds an `OR_ADD` `ClientOp` from a stored OR-Map entry.
///
/// Carries the entry's value, tag, and timestamp so the destination merges it
/// via add-wins semantics (`or_record: Some(Some(_))`).
fn or_add_client_op(map: &str, key: &str, entry: &OrMapEntry) -> ClientOp {
    ClientOp {
        id: Some(format!("{map}/{key}/{}", entry.tag)),
        map_name: map.to_string(),
        key: key.to_string(),
        op_type: None,
        record: None,
        or_record: Some(Some(ORMapRecord {
            value: value_to_rmpv(&entry.value),
            timestamp: entry.timestamp.clone(),
            tag: entry.tag.clone(),
            ttl_ms: None,
        })),
        or_tag: Some(Some(entry.tag.clone())),
        write_concern: None,
        timeout: None,
    }
}

/// Builds an `OR_REMOVE` `ClientOp` for a tombstoned tag.
///
/// An `OR_REMOVE` carries no value: `or_record: None` + `or_tag: Some(Some(tag))`
/// is the shape the CRDT merge path classifies as a tag-based removal. Without
/// this, a removal observed on the source node could never propagate — the
/// destination would keep resurrecting the removed tag, defeating convergence.
fn or_remove_client_op(map: &str, key: &str, tag: &str) -> ClientOp {
    ClientOp {
        id: Some(format!("{map}/{key}/{tag}#remove")),
        map_name: map.to_string(),
        key: key.to_string(),
        op_type: None,
        record: None,
        or_record: None,
        or_tag: Some(Some(tag.to_string())),
        write_concern: None,
        timeout: None,
    }
}

// ---------------------------------------------------------------------------
// SimCluster
// ---------------------------------------------------------------------------

/// Orchestrates N `SimNode` instances in a single process for simulation testing.
///
/// Provides convenience methods for writes, reads, time advancement,
/// and fault injection. All nodes share the same `SimTransport` for
/// inter-node communication.
pub struct SimCluster {
    /// All nodes in the cluster (some may be dead).
    pub nodes: Vec<SimNode>,
    /// Network fault injection layer (shared with transport for partition checks).
    pub network: Arc<SimNetwork>,
    /// Shared transport for inter-node message delivery.
    transport: SimTransport,
    /// Number of nodes to create.
    node_count: usize,
    /// Seed for deterministic RNG.
    pub seed: u64,
    /// Whether the cluster has been started.
    started: bool,
}

impl SimCluster {
    /// Creates a new `SimCluster` configuration. Call `start()` to build nodes.
    #[must_use]
    pub fn new(node_count: usize, seed: u64) -> Self {
        let network = Arc::new(SimNetwork::new());
        Self {
            nodes: Vec::with_capacity(node_count),
            transport: SimTransport::new(Arc::clone(&network)),
            network,
            node_count,
            seed,
            started: false,
        }
    }

    /// Builds all `SimNode` instances, registers them with the shared transport,
    /// and marks the cluster as started.
    ///
    /// # Errors
    ///
    /// Returns an error if any `SimNode::build()` call fails.
    pub fn start(&mut self) -> anyhow::Result<()> {
        for i in 0..self.node_count {
            let node_id = format!("sim-node-{i}");
            let node = SimNode::build(&node_id, self.seed, self.transport.clone())?;
            self.transport
                .register(&node_id, Arc::clone(&node.crdt_service));
            self.nodes.push(node);
        }
        self.started = true;
        Ok(())
    }

    /// Simulates a node crash by marking it as dead and unregistering
    /// its `CrdtService` from the transport.
    pub fn kill_node(&mut self, idx: usize) {
        if let Some(node) = self.nodes.get_mut(idx) {
            self.transport.unregister(&node.node_id);
            node.kill();
        }
    }

    /// Restarts a crashed node with fresh state (new service stack).
    ///
    /// # Errors
    ///
    /// Returns an error if `SimNode::build()` fails.
    pub fn restart_node(&mut self, idx: usize) -> anyhow::Result<()> {
        let node_id = format!("sim-node-{idx}");
        let node = SimNode::build(&node_id, self.seed, self.transport.clone())?;
        self.transport
            .register(&node_id, Arc::clone(&node.crdt_service));
        if idx < self.nodes.len() {
            self.nodes[idx] = node;
        }
        Ok(())
    }

    /// Writes a value to a specific node via the Tower `Service<Operation>` interface.
    ///
    /// Constructs `Operation::ClientOp` with `connection_id: None` to skip
    /// client auth/ACL validation (correct for simulation).
    ///
    /// # Errors
    ///
    /// Returns an error if the node index is out of range, the node is dead,
    /// or the CRDT service rejects the operation.
    pub async fn write(
        &self,
        node_idx: usize,
        map: &str,
        key: &str,
        value: rmpv::Value,
    ) -> anyhow::Result<()> {
        let node = self
            .nodes
            .get(node_idx)
            .ok_or_else(|| anyhow::anyhow!("node index {node_idx} out of range"))?;

        if !node.is_alive() {
            return Err(anyhow::anyhow!("node {node_idx} is dead"));
        }

        let partition_id = topgun_core::hash_to_partition(key);
        let ts = Timestamp {
            millis: 0,
            counter: 0,
            node_id: node.node_id.clone(),
        };
        let mut ctx = OperationContext::new(0, service_names::CRDT, ts, 5000);
        ctx.partition_id = Some(partition_id);
        ctx.caller_origin = CallerOrigin::System;
        // connection_id remains None -- skips auth/validation

        let lww_record = LWWRecord {
            value: Some(value),
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: node.node_id.clone(),
            },
            ttl_ms: None,
        };

        let client_op = ClientOp {
            id: Some(format!("{map}/{key}")),
            map_name: map.to_string(),
            key: key.to_string(),
            op_type: None,
            record: Some(Some(lww_record)),
            or_record: None,
            or_tag: None,
            write_concern: None,
            timeout: None,
        };

        let op = Operation::ClientOp {
            ctx,
            payload: ClientOpMessage { payload: client_op },
        };

        let mut svc = Arc::clone(&node.crdt_service);
        Service::call(&mut svc, op).await?;

        Ok(())
    }

    /// Reads a value from a specific node's `RecordStore`.
    ///
    /// # Errors
    ///
    /// Returns an error if the node index is out of range, the node is dead,
    /// or the storage read fails.
    pub async fn read(
        &self,
        node_idx: usize,
        map: &str,
        key: &str,
    ) -> anyhow::Result<Option<RecordValue>> {
        let node = self
            .nodes
            .get(node_idx)
            .ok_or_else(|| anyhow::anyhow!("node index {node_idx} out of range"))?;

        if !node.is_alive() {
            return Err(anyhow::anyhow!("node {node_idx} is dead"));
        }

        let partition_id = topgun_core::hash_to_partition(key);
        let store = node.record_store_factory.get_or_create(map, partition_id);
        let record = store.get(key, false).await?;

        Ok(record.map(|r| r.value))
    }

    /// Fast-forwards virtual time via madsim's time API.
    pub async fn advance_time(&self, duration: Duration) {
        super::time::sleep(duration).await;
    }

    /// Injects a network partition between two groups of nodes.
    pub fn inject_partition(&self, nodes_a: &[usize], nodes_b: &[usize]) {
        let ids_a: Vec<String> = nodes_a
            .iter()
            .filter_map(|&i| self.nodes.get(i).map(|n| n.node_id.clone()))
            .collect();
        let ids_b: Vec<String> = nodes_b
            .iter()
            .filter_map(|&i| self.nodes.get(i).map(|n| n.node_id.clone()))
            .collect();
        self.network.inject_partition(&ids_a, &ids_b);
    }

    /// Heals all network partitions.
    pub fn heal_partition(&self) {
        self.network.heal_partition();
    }

    /// Propagates the current value of `(map, key)` from every alive node to
    /// every other alive node via `SimTransport::deliver()`.
    ///
    /// For each alive node that has a value for the key, an `OpBatchMessage`
    /// is constructed and delivered to all other alive nodes. Delivery respects
    /// partition state — partitioned links are silently dropped by `SimTransport`.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from any alive node's store fails, or if
    /// `deliver()` returns an error.
    pub async fn sync_all(&self, map: &str, key: &str) -> anyhow::Result<()> {
        let partition_id = topgun_core::hash_to_partition(key);

        // Collect (node_id, ClientOp) for every alive node that has the key.
        let mut entries: Vec<(String, ClientOp)> = Vec::new();

        for node in &self.nodes {
            if !node.is_alive() {
                continue;
            }

            let store = node.record_store_factory.get_or_create(map, partition_id);
            let record = store.get(key, false).await?;

            if let Some(rec) = record {
                match rec.value {
                    RecordValue::Lww { value, timestamp } => {
                        let lww = LWWRecord {
                            value: Some(value_to_rmpv(&value)),
                            timestamp,
                            ttl_ms: None,
                        };
                        entries.push((
                            node.node_id.clone(),
                            ClientOp {
                                id: Some(format!("{map}/{key}")),
                                map_name: map.to_string(),
                                key: key.to_string(),
                                op_type: None,
                                record: Some(Some(lww)),
                                or_record: None,
                                or_tag: None,
                                write_concern: None,
                                timeout: None,
                            },
                        ));
                    }
                    RecordValue::OrMap {
                        records,
                        tombstones,
                    } => {
                        // Carry the full OR-Map state: every live entry as an
                        // OR_ADD and every tombstone as an OR_REMOVE. Forwarding
                        // only `records[0]` (the old behavior) made a two-node
                        // OR_REMOVE convergence scenario inexpressible — removals
                        // never crossed the wire, so a removed tag silently
                        // resurrected on the peer.
                        for entry in &records {
                            entries.push((node.node_id.clone(), or_add_client_op(map, key, entry)));
                        }
                        for tag in &tombstones {
                            entries
                                .push((node.node_id.clone(), or_remove_client_op(map, key, tag)));
                        }
                    }
                    RecordValue::OrTombstones { tags } => {
                        // Legacy tombstone-only blob: propagate each removal so a
                        // peer still holding the tag drops it (remove-wins).
                        for tag in &tags {
                            entries
                                .push((node.node_id.clone(), or_remove_client_op(map, key, tag)));
                        }
                    }
                }
            }
        }

        // For each source node that has data, deliver to every other alive node.
        for (from_node_id, client_op) in entries {
            let batch = OpBatchMessage {
                payload: OpBatchPayload {
                    ops: vec![client_op],
                    write_concern: None,
                    timeout: None,
                },
            };
            self.transport.deliver(&from_node_id, batch).await?;
        }

        Ok(())
    }

    /// Performs a record-level Merkle delta sync from node `src_idx` to node `dst_idx`
    /// for the given map.
    ///
    /// Uses partition 0 as the aggregate (per the dual-write pattern established by
    /// the Merkle partition-mismatch fix): reads all keys from the source node's
    /// partition-0 store and sends any that are absent or have an older timestamp on
    /// the destination as an `OpBatchMessage`.
    ///
    /// Partition state is respected: if the src–dst link is partitioned the batch
    /// is silently dropped by `SimTransport::deliver()`.
    ///
    /// # Errors
    ///
    /// Returns an error if either node index is out of range, if either node is
    /// dead, or if reading from any store fails.
    #[allow(clippy::too_many_lines)]
    pub async fn merkle_sync_pair(
        &self,
        src_idx: usize,
        dst_idx: usize,
        map: &str,
    ) -> anyhow::Result<()> {
        let src = self
            .nodes
            .get(src_idx)
            .ok_or_else(|| anyhow::anyhow!("src node index {src_idx} out of range"))?;
        let dst = self
            .nodes
            .get(dst_idx)
            .ok_or_else(|| anyhow::anyhow!("dst node index {dst_idx} out of range"))?;

        if !src.is_alive() {
            return Err(anyhow::anyhow!("src node {src_idx} is dead"));
        }
        if !dst.is_alive() {
            return Err(anyhow::anyhow!("dst node {dst_idx} is dead"));
        }

        let src_node_id = src.node_id.clone();
        let dst_node_id = dst.node_id.clone();

        // Skip delivery early if the link is currently partitioned.
        if self.network.is_partitioned(&src_node_id, &dst_node_id) {
            return Ok(());
        }

        // Read all records across all partitions for this map on both src and dst.
        // Data is distributed across partitions by key hash, so we must scan every
        // partition rather than a single aggregate partition.
        let src_stores = src.record_store_factory.get_all_for_map(map);
        let dst_stores = dst.record_store_factory.get_all_for_map(map);

        // Collect all records from source across all partitions.
        let mut src_records: Vec<(String, RecordValue)> = Vec::new();
        for src_store in &src_stores {
            src_store.for_each_boxed(
                &mut |key, record| {
                    src_records.push((key.to_string(), record.value.clone()));
                },
                false,
            );
        }

        // Collect destination timestamps for LWW records to detect stale entries.
        let mut dst_timestamps: std::collections::HashMap<String, Timestamp> =
            std::collections::HashMap::new();
        for dst_store in &dst_stores {
            dst_store.for_each_boxed(
                &mut |key, record| {
                    if let RecordValue::Lww { timestamp, .. } = &record.value {
                        dst_timestamps.insert(key.to_string(), timestamp.clone());
                    }
                },
                false,
            );
        }

        // Build ops for records missing or older on destination.
        let mut ops: Vec<ClientOp> = Vec::new();

        for (key, value) in src_records {
            let client_op = match &value {
                RecordValue::Lww {
                    value: v,
                    timestamp,
                } => {
                    // Skip if destination already has an equal or newer timestamp.
                    // Full Timestamp ordering includes node_id as tiebreaker (millis → counter → node_id),
                    // matching the LWW semantics used by LWWMap::merge().
                    if let Some(dst_ts) = dst_timestamps.get(&key) {
                        if dst_ts >= timestamp {
                            continue;
                        }
                    }
                    ClientOp {
                        id: Some(format!("{map}/{key}")),
                        map_name: map.to_string(),
                        key: key.clone(),
                        op_type: None,
                        record: Some(Some(LWWRecord {
                            value: Some(value_to_rmpv(v)),
                            timestamp: timestamp.clone(),
                            ttl_ms: None,
                        })),
                        or_record: None,
                        or_tag: None,
                        write_concern: None,
                        timeout: None,
                    }
                }
                RecordValue::OrMap {
                    records,
                    tombstones,
                } => {
                    // Transfer the full OR-Map state: every live entry as an
                    // OR_ADD and every tombstone as an OR_REMOVE. The destination
                    // merges via CRDT semantics (add-wins / remove-wins). Dropping
                    // tombstones here would let a removed tag resurrect on the
                    // peer — the exact gap that hid the OR_REMOVE clobber bug.
                    for entry in records {
                        ops.push(or_add_client_op(map, &key, entry));
                    }
                    for tag in tombstones {
                        ops.push(or_remove_client_op(map, &key, tag));
                    }
                    continue;
                }
                RecordValue::OrTombstones { tags } => {
                    // Legacy tombstone-only blob: propagate each removal so a peer
                    // still holding the tag drops it (remove-wins).
                    for tag in tags {
                        ops.push(or_remove_client_op(map, &key, tag));
                    }
                    continue;
                }
            };
            ops.push(client_op);
        }

        if ops.is_empty() {
            return Ok(());
        }

        let batch = OpBatchMessage {
            payload: OpBatchPayload {
                ops,
                write_concern: None,
                timeout: None,
            },
        };

        // Deliver directly to the destination node's CrdtService (targeted delivery).
        let dst_node = &self.nodes[dst_idx];
        let ts = Timestamp {
            millis: 0,
            counter: 0,
            node_id: src_node_id,
        };
        let ctx = OperationContext::new(0, service_names::CRDT, ts, 5000);
        let op = Operation::OpBatch {
            ctx,
            payload: batch,
        };
        let mut svc = Arc::clone(&dst_node.crdt_service);
        let _ = Service::call(&mut svc, op).await;

        Ok(())
    }

    /// Adds a new node to a running cluster and registers it with the transport.
    ///
    /// The new node gets a `node_id` of `"sim-node-N"` where `N` is the current
    /// length of `self.nodes`. This is the correct ID format for late-joiner tests.
    ///
    /// # Errors
    ///
    /// Returns an error if `SimNode::build()` fails.
    pub fn add_node(&mut self) -> anyhow::Result<usize> {
        let node_id = format!("sim-node-{}", self.nodes.len());
        let node = SimNode::build(&node_id, self.seed, self.transport.clone())?;
        self.transport
            .register(&node_id, Arc::clone(&node.crdt_service));
        self.nodes.push(node);
        Ok(self.nodes.len() - 1)
    }

    /// Writes an OR-Map entry to a specific node.
    ///
    /// Uses the same path as `write()` but constructs a `ClientOp` with
    /// `or_record: Some(Some(...))` and `or_tag: Some(Some(tag))`, enabling
    /// OR-Map concurrent-add semantics (each entry is uniquely tagged).
    ///
    /// # Errors
    ///
    /// Returns an error if the node index is out of range, the node is dead,
    /// or the CRDT service rejects the operation.
    pub async fn or_write(
        &self,
        node_idx: usize,
        map: &str,
        key: &str,
        tag: impl Into<String>,
        value: rmpv::Value,
    ) -> anyhow::Result<()> {
        let tag = tag.into();
        let node = self
            .nodes
            .get(node_idx)
            .ok_or_else(|| anyhow::anyhow!("node index {node_idx} out of range"))?;

        if !node.is_alive() {
            return Err(anyhow::anyhow!("node {node_idx} is dead"));
        }

        let partition_id = topgun_core::hash_to_partition(key);
        let ts = Timestamp {
            millis: 0,
            counter: 0,
            node_id: node.node_id.clone(),
        };
        let mut ctx = OperationContext::new(0, service_names::CRDT, ts.clone(), 5000);
        ctx.partition_id = Some(partition_id);
        ctx.caller_origin = CallerOrigin::System;

        let or_record = ORMapRecord {
            value,
            timestamp: ts,
            tag: tag.clone(),
            ttl_ms: None,
        };

        let client_op = ClientOp {
            id: Some(format!("{map}/{key}/{tag}")),
            map_name: map.to_string(),
            key: key.to_string(),
            op_type: None,
            record: None,
            or_record: Some(Some(or_record)),
            or_tag: Some(Some(tag)),
            write_concern: None,
            timeout: None,
        };

        let op = Operation::ClientOp {
            ctx,
            payload: ClientOpMessage { payload: client_op },
        };

        let mut svc = Arc::clone(&node.crdt_service);
        Service::call(&mut svc, op).await?;

        Ok(())
    }

    /// Removes an OR-Map entry (by tag) from a specific node.
    ///
    /// Mirrors [`or_write`](Self::or_write) but constructs the `OR_REMOVE` shape:
    /// `or_record: None` + `or_tag: Some(Some(tag))`. The CRDT merge path applies
    /// this as a tag-based, remove-wins deletion — dropping the matched tag from
    /// the live entry set and recording it as a tombstone, while preserving every
    /// concurrent survivor. The removal need not have been observed locally first:
    /// tombstoning an unseen tag still suppresses a later-arriving add.
    ///
    /// # Errors
    ///
    /// Returns an error if the node index is out of range, the node is dead,
    /// or the CRDT service rejects the operation.
    pub async fn or_remove(
        &self,
        node_idx: usize,
        map: &str,
        key: &str,
        tag: impl Into<String>,
    ) -> anyhow::Result<()> {
        let tag = tag.into();
        let node = self
            .nodes
            .get(node_idx)
            .ok_or_else(|| anyhow::anyhow!("node index {node_idx} out of range"))?;

        if !node.is_alive() {
            return Err(anyhow::anyhow!("node {node_idx} is dead"));
        }

        let partition_id = topgun_core::hash_to_partition(key);
        let ts = Timestamp {
            millis: 0,
            counter: 0,
            node_id: node.node_id.clone(),
        };
        let mut ctx = OperationContext::new(0, service_names::CRDT, ts, 5000);
        ctx.partition_id = Some(partition_id);
        ctx.caller_origin = CallerOrigin::System;

        let client_op = or_remove_client_op(map, key, &tag);

        let op = Operation::ClientOp {
            ctx,
            payload: ClientOpMessage { payload: client_op },
        };

        let mut svc = Arc::clone(&node.crdt_service);
        Service::call(&mut svc, op).await?;

        Ok(())
    }

    /// Asserts that all alive nodes hold the same value for `(map, key)`.
    ///
    /// Collects the stored `RecordValue` from every alive node and panics with
    /// a descriptive message if any two nodes disagree. Returns the agreed-upon
    /// value, or `None` if all alive nodes agree the key is absent.
    ///
    /// # Panics
    ///
    /// Panics if any two alive nodes hold different values for the same key.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from any alive node's store fails.
    pub async fn assert_converged(
        &self,
        map: &str,
        key: &str,
    ) -> anyhow::Result<Option<RecordValue>> {
        let mut first: Option<(usize, Option<RecordValue>)> = None;

        for (idx, node) in self.nodes.iter().enumerate() {
            if !node.is_alive() {
                continue;
            }

            let partition_id = topgun_core::hash_to_partition(key);
            let store = node.record_store_factory.get_or_create(map, partition_id);
            let record = store.get(key, false).await?;
            let value = record.map(|r| r.value);

            match &first {
                None => {
                    first = Some((idx, value));
                }
                Some((first_idx, first_value)) => {
                    // Compare serialized forms because RecordValue does not implement PartialEq.
                    let lhs = rmp_serde::to_vec_named(first_value).unwrap_or_default();
                    let rhs = rmp_serde::to_vec_named(&value).unwrap_or_default();
                    assert_eq!(
                        lhs, rhs,
                        "convergence failure for map={map:?} key={key:?}: \
                         node {first_idx} and node {idx} hold different values",
                    );
                }
            }
        }

        Ok(first.and_then(|(_, v)| v))
    }

    /// Drives a structured query against a specific node through the SAME
    /// DAG execution path a real WebSocket client hits, returning the
    /// result rows.
    ///
    /// This method drives `coordinator.execute_distributed` directly to exercise
    /// the DAG execution engine (and its single-node bypass) under fault injection.
    /// Note: it does NOT go through `classify` or `QueryService::handle_query_subscribe`
    /// — it tests the engine, not the production routing/handler wiring. End-to-end
    /// routing is covered by the TS integration suite and the classify unit tests.
    ///
    /// # Row-key note
    /// For non-GROUP-BY queries, the DAG result rows do not carry a `__key`
    /// field (that field is GROUP-BY-specific). Returned `QueryResultEntry`
    /// values therefore use synthetic `"row-{i}"` keys. Callers should assert
    /// on `.value` content, ordering, and length — not on `.key` identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the node index is out of range, the node is dead,
    /// the DAG execution fails, or the coordinator returns an error.
    pub async fn query(
        &self,
        node_idx: usize,
        map_name: &str,
        query: Query,
    ) -> anyhow::Result<Vec<QueryResultEntry>> {
        let node = self
            .nodes
            .get(node_idx)
            .ok_or_else(|| anyhow::anyhow!("node index {node_idx} out of range"))?;

        if !node.is_alive() {
            return Err(anyhow::anyhow!("node {node_idx} is dead"));
        }

        // Drive coordinator.execute_distributed directly. The coordinator's
        // single-node bypass routes this through execute_local → run_dag_local →
        // DagExecutor without needing cluster fan-out or a connection context
        // (execute_distributed takes only the query + map name, no auth ctx).
        let dist_result = node
            .coordinator
            .execute_distributed(&query, map_name)
            .await?;

        // Map raw rows to QueryResultEntry. Non-GROUP-BY DAG rows do not carry
        // a `__key` field (that is GROUP-BY-specific). Use synthetic row keys
        // so callers can identify entries; assert on .value content, not .key.
        let results: Vec<QueryResultEntry> = dist_result
            .rows
            .into_iter()
            .enumerate()
            .map(|(i, val)| {
                // Prefer the `__key` field if present (GROUP-BY result), fall
                // back to a synthetic index key for filter/sort/limit results.
                let key = if let rmpv::Value::Map(ref pairs) = val {
                    pairs.iter().find_map(|(k, v)| {
                        if k.as_str() == Some("__key") {
                            v.as_str().map(str::to_string)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
                .unwrap_or_else(|| format!("row-{i}"));
                QueryResultEntry { key, value: val }
            })
            .collect();

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::manual_is_multiple_of,
    clippy::items_after_statements
)]
mod tests {
    use std::time::Duration;

    use tokio::sync::{mpsc, oneshot};

    use crate::cluster::dispatch::handle_cluster_peer_frame;
    use crate::cluster::messages::{ClusterMessage, DagCompletePayload, DagExecutePayload};
    use crate::cluster::state::InboundClusterMessage;
    use crate::cluster::types::{MemberInfo, MembersView, NodeState};
    use crate::dag::types::{
        DagPlanDescriptor, ExecutionPlan, ProcessorType, QueryConfig, VertexDescriptor,
    };
    use crate::network::config::ConnectionConfig;
    use crate::network::connection::{ConnectionKind, OutboundMessage};
    use crate::storage::record::RecordValue;
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};

    use super::*;

    /// Sets up membership on a SimNode's ClusterState so the coordinator sees
    /// the given node IDs as active members.
    fn set_membership(node: &SimNode, node_ids: &[&str]) {
        let members: Vec<MemberInfo> = node_ids
            .iter()
            .enumerate()
            .map(|(i, nid)| MemberInfo {
                node_id: nid.to_string(),
                host: "127.0.0.1".to_string(),
                client_port: 9000 + i as u16,
                cluster_port: 9100 + i as u16,
                state: NodeState::Active,
                join_version: 1,
            })
            .collect();
        node.cluster_state.update_view(MembersView {
            version: 1,
            members,
        });
    }

    /// Assigns partition ownership on a SimNode's cluster state. The closure
    /// receives the partition ID and returns the owner node_id.
    fn assign_partitions(node: &SimNode, owner_fn: &dyn Fn(u32) -> String) {
        let count = node.cluster_state.partition_table.partition_count();
        for pid in 0..count {
            node.cluster_state
                .partition_table
                .set_owner(pid, owner_fn(pid), Vec::new());
        }
    }

    /// Creates a ClusterPeer connection on `from_node` targeting `to_node_id`
    /// and spawns a bridge task that forwards outbound binary frames to
    /// `to_inbound_tx` via `handle_cluster_peer_frame`.
    ///
    /// Returns a JoinHandle for the bridge task (should be aborted on cleanup).
    async fn bridge_peer_connection(
        from_node: &SimNode,
        to_node_id: &str,
        to_inbound_tx: mpsc::Sender<InboundClusterMessage>,
    ) -> tokio::task::JoinHandle<()> {
        let config = ConnectionConfig::default();
        let (handle, mut outbound_rx) = from_node
            .connection_registry
            .register(ConnectionKind::ClusterPeer, &config);

        // Set the peer_node_id metadata so send_to_peer can find this connection.
        {
            let mut meta = handle.metadata.write().await;
            meta.peer_node_id = Some(to_node_id.to_string());
        }

        let from_node_id = from_node.node_id.clone();
        tokio::spawn(async move {
            while let Some(msg) = outbound_rx.recv().await {
                if let OutboundMessage::Binary(bytes) = msg {
                    // Forward the binary frame to the target node's dispatch loop.
                    let _ = handle_cluster_peer_frame(&bytes, from_node_id.clone(), &to_inbound_tx);
                }
            }
        })
    }

    // -----------------------------------------------------------------------
    // AC2 + AC3: DagExecute routes to handle_dag_execute on peer, DagComplete
    // arrives back at coordinator and resolves the completion_registry oneshot.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn dag_execute_roundtrip_resolves_completion() {
        let mut cluster = SimCluster::new(2, 42);
        cluster.start().expect("cluster start");

        let node_ids = ["sim-node-0", "sim-node-1"];

        // Set membership so both nodes are visible to the coordinator.
        for node in &cluster.nodes {
            set_membership(node, &node_ids);
        }

        // Assign partitions: even partitions → node-0, odd → node-1.
        let owner_fn = |pid: u32| -> String {
            if pid % 2 == 0 {
                "sim-node-0".to_string()
            } else {
                "sim-node-1".to_string()
            }
        };
        for node in &cluster.nodes {
            assign_partitions(node, &owner_fn);
        }

        // Bridge connections: node-0 ↔ node-1. Each node can send binary
        // messages to the other's dispatch loop.
        let bridge_0_to_1 = bridge_peer_connection(
            &cluster.nodes[0],
            "sim-node-1",
            cluster.nodes[1].inbound_tx.clone(),
        )
        .await;
        let bridge_1_to_0 = bridge_peer_connection(
            &cluster.nodes[1],
            "sim-node-0",
            cluster.nodes[0].inbound_tx.clone(),
        )
        .await;

        // Write a record to node-1's storage so DagExecute has data to process.
        let map_name = "test_map";
        let key = "user-1";
        let partition_id = topgun_core::hash_to_partition(key);
        let store = cluster.nodes[1]
            .record_store_factory
            .get_or_create(map_name, partition_id);
        store
            .put(
                key,
                RecordValue::Lww {
                    value: topgun_core::Value::Int(42),
                    timestamp: Timestamp {
                        millis: 100,
                        counter: 0,
                        node_id: "sim-node-1".to_string(),
                    },
                },
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .expect("put should succeed");

        // Build a minimal execution plan with a scan vertex only.
        let execution_id = "test-exec-1".to_string();
        let descriptor = DagPlanDescriptor {
            vertices: vec![VertexDescriptor {
                name: "scan".to_string(),
                local_parallelism: 1,
                processor_type: ProcessorType::Scan,
                preferred_partitions: None,
                config: Some(rmpv::Value::Map(vec![(
                    rmpv::Value::String("mapName".into()),
                    rmpv::Value::String(map_name.into()),
                )])),
            }],
            edges: Vec::new(),
        };

        let mut partition_assignment = std::collections::HashMap::new();
        partition_assignment.insert("sim-node-1".to_string(), vec![partition_id]);

        let plan = ExecutionPlan {
            plan: descriptor,
            partition_assignment,
            version: 1,
            config: QueryConfig::default(),
            created_at: 0,
        };

        let plan_bytes = rmp_serde::to_vec_named(&plan).expect("serialize plan");
        let dag_execute = DagExecutePayload {
            execution_id: execution_id.clone(),
            plan: plan_bytes,
        };

        // Register a completion entry on node-0 for the expected response.
        let (oneshot_tx, oneshot_rx) = oneshot::channel::<DagCompletePayload>();
        let completion_key = format!("{execution_id}:sim-node-1");
        cluster.nodes[0]
            .completion_registry
            .insert(completion_key.clone(), oneshot_tx);

        // Serialize DagExecute and inject into node-1's dispatch loop.
        let msg = ClusterMessage::DagExecute(dag_execute);
        let msg_bytes = rmp_serde::to_vec_named(&msg).expect("serialize msg");
        handle_cluster_peer_frame(
            &msg_bytes,
            "sim-node-0".to_string(),
            &cluster.nodes[1].inbound_tx,
        )
        .expect("frame accepted");

        // The full roundtrip:
        // 1. node-1 dispatch loop receives DagExecute, spawns handle_dag_execute
        // 2. handle_dag_execute runs the scan, builds DagComplete
        // 3. handle_dag_execute sends DagComplete via node-1's ConnectionRegistry
        // 4. Bridge task forwards the binary to node-0's dispatch loop
        // 5. node-0 dispatch loop receives DagComplete, calls handle_dag_complete
        // 6. handle_dag_complete resolves the oneshot in completion_registry

        let result = tokio::time::timeout(Duration::from_secs(5), oneshot_rx)
            .await
            .expect("should not timeout")
            .expect("oneshot should resolve");

        assert_eq!(result.execution_id, execution_id);
        assert_eq!(result.node_id, "sim-node-1");
        assert!(result.success, "execution should succeed");
        assert!(result.results.is_some(), "should have result bytes");

        // Cleanup bridge tasks.
        bridge_0_to_1.abort();
        bridge_1_to_0.abort();
    }

    // -----------------------------------------------------------------------
    // SimCluster::query — filter + multi-field sort + limit via DAG path
    //
    // This test drives a structured query through the production
    // classify → DAG path (via coordinator.execute_distributed) and asserts
    // the result rows are correctly filtered, sorted, and limit-clamped.
    // The assertions would FAIL if SimCluster::query read the record store
    // directly or if the DAG were bypassed.
    // -----------------------------------------------------------------------

    /// Builds an object record `{ "score": n }` for query tests. Records carry a
    /// real field so the DAG Filter/Sort stages operate on actual field names
    /// (matching production records, which are objects).
    fn score_record(n: i64) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String("score".into()),
            rmpv::Value::Integer(n.into()),
        )])
    }

    /// Extracts the `score` field from a DAG result row for assertion purposes.
    fn get_int_field(val: &rmpv::Value) -> Option<i64> {
        if let rmpv::Value::Map(pairs) = val {
            for (k, v) in pairs {
                if k.as_str() == Some("score") {
                    return match v {
                        rmpv::Value::Integer(i) => i.as_i64(),
                        _ => None,
                    };
                }
            }
        }
        None
    }

    #[tokio::test]
    #[cfg(feature = "simulation")]
    async fn query_filter_sort_limit_routes_through_dag() {
        let mut cluster = SimCluster::new(1, 0);
        cluster.start().expect("cluster start");

        let node_ids = ["sim-node-0"];
        let map_name = "scores";

        // Give the coordinator a complete view of the single node so the
        // single-node bypass engages (needs_distribution → false, routes to
        // execute_local → DagExecutor).
        set_membership(&cluster.nodes[0], &node_ids);
        assign_partitions(&cluster.nodes[0], &|_| "sim-node-0".to_string());

        // Write 5 records: values 3, 5, 7, 10, 15.
        // After storage round-trip via CrdtService + ScanProcessor, each
        // record becomes rmpv::Value::Map([("score", Integer(n))]).
        for (key, val) in [
            ("rec-a", 3i64),
            ("rec-b", 5i64),
            ("rec-c", 7i64),
            ("rec-d", 10i64),
            ("rec-e", 15i64),
        ] {
            cluster
                .write(0, map_name, key, score_record(val))
                .await
                .expect("write should succeed");
        }

        // Build a query: filter score >= 5, sort by "score" Asc (two SortField
        // entries exercise the multi-field sort wire format and the DAG
        // converter's multi-field sort plan), limit 3.
        use topgun_core::messages::base::{PredicateNode, PredicateOp, SortDirection, SortField};
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Gte,
                attribute: Some("score".to_string()),
                value: Some(rmpv::Value::Integer(5i64.into())),
                children: None,
                value_ref: None,
            }),
            sort: Some(vec![
                // Two sort fields exercise multi-field sort code path in the
                // DAG converter and SortProcessor.
                SortField {
                    field: "score".to_string(),
                    direction: SortDirection::Asc,
                },
                SortField {
                    field: "score".to_string(),
                    direction: SortDirection::Asc,
                },
            ]),
            limit: Some(3),
            ..Default::default()
        };

        let results = cluster
            .query(0, map_name, query)
            .await
            .expect("query should succeed");

        // AC1(a): limit-clamped to 3 rows.
        assert_eq!(results.len(), 3, "limit 3 should return exactly 3 rows");

        // AC1(b): correctly filtered — "score" == 3 must be absent.
        let has_three = results.iter().any(|r| get_int_field(&r.value) == Some(3));
        assert!(!has_three, "record with Int=3 should be excluded by filter");

        // AC1(c): correctly sorted ascending AND limit-clamped.
        // "score"=15 must be absent (it would be 4th after ascending sort).
        let has_fifteen = results.iter().any(|r| get_int_field(&r.value) == Some(15));
        assert!(
            !has_fifteen,
            "record with Int=15 should be cut off by limit"
        );

        // AC1(d): multi-field ordering assertion — the first result MUST be
        // the smallest value (score=5) after ascending sort.
        //
        // VACUITY GUARD: Flipping the expected order here — e.g., asserting
        // items[0] has score=15 — MUST make this test FAIL. The ascending order
        // comes from the DAG SortProcessor, not a test-side sort. If the sort
        // were removed, records would arrive in insertion / hash-partition
        // order, not ascending order, and this assertion would break.
        let first_int = get_int_field(&results[0].value);
        assert_eq!(
            first_int,
            Some(5),
            "first result should be Int=5 (smallest after filter, ascending sort)"
        );

        let second_int = get_int_field(&results[1].value);
        assert_eq!(
            second_int,
            Some(7),
            "second result should be Int=7 (ascending sort, engine-driven)"
        );

        let third_int = get_int_field(&results[2].value);
        assert_eq!(
            third_int,
            Some(10),
            "third result should be Int=10 (ascending sort, engine-driven)"
        );
    }

    // -----------------------------------------------------------------------
    // AC2: fault injection — query against a still-alive node under partition.
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[cfg(feature = "simulation")]
    async fn query_under_partition_returns_alive_node_results() {
        // Two-node cluster. After partition, we query node-0 (still alive)
        // and verify it returns its own local data via the DAG path.
        let mut cluster = SimCluster::new(2, 1);
        cluster.start().expect("cluster start");

        let map_name = "fault_map";

        // Each node sees only itself as an active member so the coordinator's
        // single-node bypass fires (needs_distribution requires >1 partition
        // assignment). This keeps queries local and avoids distributed fan-out
        // which would require live ClusterPeer connections — out of scope here.
        set_membership(&cluster.nodes[0], &["sim-node-0"]);
        set_membership(&cluster.nodes[1], &["sim-node-1"]);
        assign_partitions(&cluster.nodes[0], &|_| "sim-node-0".to_string());
        assign_partitions(&cluster.nodes[1], &|_| "sim-node-1".to_string());

        // Write a record only to node-0.
        cluster
            .write(0, map_name, "key-alive", score_record(42))
            .await
            .expect("write to node-0");

        // Inject a network partition between node-0 and node-1.
        cluster.inject_partition(&[0], &[1]);

        // Query node-0 (still alive, owns its own data). Even though node-1
        // is partitioned away, node-0's single-node coordinator bypass runs
        // locally and returns its own records.
        use topgun_core::messages::base::{SortDirection, SortField};
        let query = Query {
            sort: Some(vec![SortField {
                field: "score".to_string(),
                direction: SortDirection::Asc,
            }]),
            ..Default::default()
        };

        let results = cluster
            .query(0, map_name, query)
            .await
            .expect("query node-0 under partition should succeed");

        // Node-0 should return the record it owns.
        assert_eq!(
            results.len(),
            1,
            "node-0 should return its own record under partition"
        );

        let int_val = get_int_field(&results[0].value);
        assert_eq!(
            int_val,
            Some(42),
            "result should be the record written to node-0"
        );

        cluster.heal_partition();
    }

    // -----------------------------------------------------------------------
    // Fault injection with filter + multi-field sort + limit.
    //
    // Key-link test that proves the structured classify → DAG routing works
    // correctly under fault. The assertions on ordering and limit-clamping can
    // ONLY be satisfied by the DAG SortProcessor and LimitProcessor — a
    // record-store-bypass would return records in insertion/hash-partition
    // order without filter or limit, causing the ordering and absence
    // assertions to fail.
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[cfg(feature = "simulation")]
    async fn sim_query_filter_sort_limit_under_node_failure() {
        // Two-node cluster. We kill node-1, then issue a filter+sort+limit
        // query against node-0. The DAG single-node bypass on node-0 executes
        // the full pipeline locally; the dead node-1 never receives the query.
        let mut cluster = SimCluster::new(2, 10);
        cluster.start().expect("cluster start");

        let map_name = "structured_fault_map";

        // Each node sees only itself as an active member so the coordinator
        // single-node bypass fires for queries on that node.
        set_membership(&cluster.nodes[0], &["sim-node-0"]);
        set_membership(&cluster.nodes[1], &["sim-node-1"]);
        assign_partitions(&cluster.nodes[0], &|_| "sim-node-0".to_string());
        assign_partitions(&cluster.nodes[1], &|_| "sim-node-1".to_string());

        // Write 6 records to node-0: values 2, 4, 6, 8, 10, 12.
        for (key, val) in [
            ("sf-a", 2i64),
            ("sf-b", 4i64),
            ("sf-c", 6i64),
            ("sf-d", 8i64),
            ("sf-e", 10i64),
            ("sf-f", 12i64),
        ] {
            cluster
                .write(0, map_name, key, score_record(val))
                .await
                .expect("write to node-0");
        }

        // Kill node-1 — it becomes unreachable.
        cluster.kill_node(1);
        assert!(!cluster.nodes[1].is_alive(), "node-1 should be dead");

        // Query node-0 (alive): filter score >= 6, sort by "score" Asc (two fields
        // to exercise the multi-field sort path), limit 2.
        // Expected result after filter: 6, 8, 10, 12.
        // After ascending sort + limit 2: [6, 8].
        use topgun_core::messages::base::{PredicateNode, PredicateOp, SortDirection, SortField};
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Gte,
                attribute: Some("score".to_string()),
                value: Some(rmpv::Value::Integer(6i64.into())),
                children: None,
                value_ref: None,
            }),
            sort: Some(vec![
                SortField {
                    field: "score".to_string(),
                    direction: SortDirection::Asc,
                },
                // Second sort field exercises the multi-field sort code path
                // in the DAG converter, matching the production wire format.
                SortField {
                    field: "score".to_string(),
                    direction: SortDirection::Asc,
                },
            ]),
            limit: Some(2),
            ..Default::default()
        };

        let results = cluster
            .query(0, map_name, query)
            .await
            .expect("query on alive node-0 under node-1 failure should succeed");

        // limit clamp: 2 rows even though 4 match the filter.
        assert_eq!(
            results.len(),
            2,
            "limit 2 should return exactly 2 rows (4 pass filter, 2 survive limit)"
        );

        // filter exclusion: records with score < 6 must be absent.
        let has_below_threshold = results
            .iter()
            .any(|r| get_int_field(&r.value).is_some_and(|v| v < 6));
        assert!(
            !has_below_threshold,
            "records with Int < 6 should be excluded by filter"
        );

        // DAG ascending sort: first result must be the smallest post-filter
        // value (score=6). A record-store fallback would return records in
        // hash-partition/insertion order, NOT ascending order, so this
        // assertion MUST fail if the DAG SortProcessor is bypassed.
        let first_int = get_int_field(&results[0].value);
        assert_eq!(
            first_int,
            Some(6),
            "first result must be Int=6 (smallest after filter, ascending DAG sort)"
        );

        // limit cutoff: score=10 and score=12 must be absent (cut off at 2).
        let has_ten_or_above = results
            .iter()
            .any(|r| get_int_field(&r.value).is_some_and(|v| v >= 10));
        assert!(
            !has_ten_or_above,
            "Int>=10 should be cut off by limit=2 after ascending DAG sort"
        );
    }

    // -----------------------------------------------------------------------
    // Global sort + global limit must hold across a true 2-node fan-out under
    // a partition/heal fault scenario.
    //
    // Each node's local stream is individually sorted but the two streams
    // interleave: node-0 holds {10,30,50} and node-1 holds {20,40,60}.
    // Correct globally-ascending order is 10,20,30,40,50,60.
    // A naive concat would produce node-0-stream ++ node-1-stream (i.e.
    // 10,30,50,20,40,60) which fails the ordering assertion, proving vacuity.
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[cfg(feature = "simulation")]
    #[allow(clippy::too_many_lines)]
    async fn distributed_merge_global_sort_and_limit_under_partition() {
        let mut cluster = SimCluster::new(2, 99);
        cluster.start().expect("cluster start");

        let node_ids = ["sim-node-0", "sim-node-1"];
        let map_name = "merge_test";

        // Both nodes must see the full membership so coordinator.execute_distributed
        // engages multi-node fan-out (needs_distribution → true).
        for node in &cluster.nodes {
            set_membership(node, &node_ids);
        }

        // Partition assignment: even partition IDs → node-0, odd → node-1.
        // This mirrors the dag_execute_roundtrip_resolves_completion pattern.
        let owner_fn = |pid: u32| -> String {
            if pid % 2 == 0 {
                "sim-node-0".to_string()
            } else {
                "sim-node-1".to_string()
            }
        };
        for node in &cluster.nodes {
            assign_partitions(node, &owner_fn);
        }

        // Bridge connections in both directions so DagExecute/DagComplete flow
        // between the coordinator node (node-0) and the peer (node-1).
        // The coordinator fans out to ALL active members including itself, so
        // node-0 also needs a self-targeting ClusterPeer connection so that
        // send_to_peer("sim-node-0", ...) can deliver to node-0's dispatch loop.
        let bridge_0_to_self = bridge_peer_connection(
            &cluster.nodes[0],
            "sim-node-0",
            cluster.nodes[0].inbound_tx.clone(),
        )
        .await;
        let bridge_0_to_1 = bridge_peer_connection(
            &cluster.nodes[0],
            "sim-node-1",
            cluster.nodes[1].inbound_tx.clone(),
        )
        .await;
        let bridge_1_to_0 = bridge_peer_connection(
            &cluster.nodes[1],
            "sim-node-0",
            cluster.nodes[0].inbound_tx.clone(),
        )
        .await;

        // Write records to the node that OWNS each key's natural partition.
        // Partition ownership: even partition IDs → node-0, odd → node-1.
        // Key names are dictated by partition hashing, not by score: each key
        // must hash to a partition whose parity routes it to the intended node,
        // so neutral names are used to avoid implying any key↔score relation.
        // Node-0 owns scores {10,30,50} and node-1 owns scores {20,40,60},
        // making correct global ascending order 10,20,30,40,50,60.
        //
        // Verified partition hashes (UTF-16 FNV-1a mod 271):
        //   "bravo"   → 168 (even → node-0)
        //   "delta"   → 142 (even → node-0)
        //   "foxtrot" → 112 (even → node-0)
        //   "alpha"   → 215 (odd  → node-1)
        //   "charlie" → 43  (odd  → node-1)
        //   "echo"    → 239 (odd  → node-1)
        let all_records: &[(&str, i64, usize)] = &[
            // (key, score, node_idx that owns the partition)
            ("bravo", 10, 0),   // partition 168 → node-0
            ("delta", 30, 0),   // partition 142 → node-0
            ("foxtrot", 50, 0), // partition 112 → node-0
            ("alpha", 20, 1),   // partition 215 → node-1
            ("charlie", 40, 1), // partition 43  → node-1
            ("echo", 60, 1),    // partition 239 → node-1
        ];

        for &(key, score, node_idx) in all_records {
            let partition_id = topgun_core::hash_to_partition(key);
            let node_id = format!("sim-node-{node_idx}");
            let store = cluster.nodes[node_idx]
                .record_store_factory
                .get_or_create(map_name, partition_id);
            store
                .put(
                    key,
                    RecordValue::Lww {
                        value: topgun_core::Value::Map(std::collections::BTreeMap::from([(
                            "score".to_string(),
                            topgun_core::Value::Int(score),
                        )])),
                        timestamp: Timestamp {
                            millis: 1_000,
                            counter: 0,
                            node_id,
                        },
                    },
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("write record to owning node's store");
        }

        // Fault scenario: inject a partition between the two nodes, then heal
        // before issuing the query. This exercises partition/heal around the
        // fan-out and confirms the coordinator handles the healed state correctly.
        cluster.inject_partition(&[0], &[1]);
        cluster.heal_partition();

        // Issue a non-GROUP-BY query with ascending sort on "score" and limit 4.
        // Global ascending order: 10,20,30,40,50,60 → limit 4 → [10,20,30,40].
        use topgun_core::messages::base::{SortDirection, SortField};
        let query = Query {
            sort: Some(vec![SortField {
                field: "score".to_string(),
                direction: SortDirection::Asc,
            }]),
            limit: Some(4),
            ..Default::default()
        };

        let dist_result = cluster.nodes[0]
            .coordinator
            .execute_distributed(&query, map_name)
            .await
            .expect("distributed query should succeed");

        // Exactly `limit` rows must be returned (sentinel absorbed internally).
        // Vacuity: naive concat without global limit could return up to 6 rows.
        assert_eq!(
            dist_result.rows.len(),
            4,
            "global limit=4 must yield exactly 4 rows, not up to N×per-node-limit"
        );
        // 6 records with limit=4 → more pages exist.
        assert!(
            dist_result.has_more,
            "has_more must be true when 6 records are present and limit=4"
        );

        // Extract scores from raw rmpv::Value results.
        let scores: Vec<i64> = dist_result
            .rows
            .iter()
            .filter_map(|v| {
                if let rmpv::Value::Map(pairs) = v {
                    pairs.iter().find_map(|(k, val)| {
                        if k.as_str() == Some("score") {
                            if let rmpv::Value::Integer(i) = val {
                                i.as_i64()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .collect();

        // Globally-correct ascending order must hold across node boundaries.
        // Vacuity: a naive concat would produce node-0's stream first (10,30,50)
        // then node-1's stream (20,40,60), and limit=4 would truncate that to
        // [10,30,50,20]; this assertion (scores == [10,20,30,40]) would FAIL
        // on a concat-style merge.
        assert_eq!(
            scores,
            vec![10, 20, 30, 40],
            "results must be in global ascending order across node boundaries"
        );

        // Cleanup bridge tasks.
        bridge_0_to_self.abort();
        bridge_0_to_1.abort();
        bridge_1_to_0.abort();
    }

    // -----------------------------------------------------------------------
    // Distributed keyset cursor under partition fault.
    //
    // Verifies that a global keyset cursor applied across a 2-node fan-out
    // returns the correct second page — strictly after the cursor position —
    // in global sort order with no duplicates and no first-page rows.
    //
    // Vacuity: a coordinator-only cursor (filtering the already-merged result
    // instead of pre-filtering on each worker) would work only when no
    // per-node limit is applied before sending — but with per-node limits,
    // coordinator-side cursor filtering would miss rows, producing pages that
    // overlap or have gaps.  A per-node-offset cursor (the stale TS blueprint)
    // would also be wrong here because each node's offset is independent of
    // the other node's data; the global keyset cursor is correct by construction.
    //
    // Coverage note: this sim drives SimCluster::query directly (which calls
    // coordinator.execute_distributed) and does NOT exercise the production
    // routing through handle_query_subscribe or the bin dispatch-loop wiring.
    // The dispatch-loop↔completion_registry link (bin:343-356) is verified by
    // source inspection per the Validation Checklist, not by this sim.
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[cfg(feature = "simulation")]
    #[allow(clippy::too_many_lines)]
    async fn distributed_keyset_cursor_under_partition() {
        use crate::query::cursor::{encode_cursor, CursorData, SortValue};
        use topgun_core::messages::base::{SortDirection, SortField};

        let mut cluster = SimCluster::new(2, 77);
        cluster.start().expect("cluster start");

        let node_ids = ["sim-node-0", "sim-node-1"];
        let map_name = "cursor_test";

        // Both nodes must see the full membership so execute_distributed engages
        // multi-node fan-out.
        for node in &cluster.nodes {
            set_membership(node, &node_ids);
        }

        // Partition assignment: even partition IDs → node-0, odd → node-1.
        let owner_fn = |pid: u32| -> String {
            if pid % 2 == 0 {
                "sim-node-0".to_string()
            } else {
                "sim-node-1".to_string()
            }
        };
        for node in &cluster.nodes {
            assign_partitions(node, &owner_fn);
        }

        // Bridge connections in both directions so DagExecute/DagComplete flow
        // between the coordinator (node-0) and the peer (node-1).
        // node-0 also needs a self-targeting connection for its own partitions.
        let bridge_0_to_self = bridge_peer_connection(
            &cluster.nodes[0],
            "sim-node-0",
            cluster.nodes[0].inbound_tx.clone(),
        )
        .await;
        let bridge_0_to_1 = bridge_peer_connection(
            &cluster.nodes[0],
            "sim-node-1",
            cluster.nodes[1].inbound_tx.clone(),
        )
        .await;
        let bridge_1_to_0 = bridge_peer_connection(
            &cluster.nodes[1],
            "sim-node-0",
            cluster.nodes[0].inbound_tx.clone(),
        )
        .await;

        // Write records with interleaved scores: node-0 owns {10,30,50}, node-1 owns {20,40,60}.
        // Global ascending order: 10,20,30,40,50,60.
        //
        // Partition hashes (UTF-16 FNV-1a mod 271) — same keys as the sort/limit test:
        //   "bravo"   → 168 (even → node-0)  score=10
        //   "delta"   → 142 (even → node-0)  score=30
        //   "foxtrot" → 112 (even → node-0)  score=50
        //   "alpha"   → 215 (odd  → node-1)  score=20
        //   "charlie" → 43  (odd  → node-1)  score=40
        //   "echo"    → 239 (odd  → node-1)  score=60
        let all_records: &[(&str, i64, usize)] = &[
            ("bravo", 10, 0),
            ("delta", 30, 0),
            ("foxtrot", 50, 0),
            ("alpha", 20, 1),
            ("charlie", 40, 1),
            ("echo", 60, 1),
        ];

        for &(key, score, node_idx) in all_records {
            let partition_id = topgun_core::hash_to_partition(key);
            let node_id = format!("sim-node-{node_idx}");
            let store = cluster.nodes[node_idx]
                .record_store_factory
                .get_or_create(map_name, partition_id);
            store
                .put(
                    key,
                    RecordValue::Lww {
                        value: topgun_core::Value::Map(std::collections::BTreeMap::from([(
                            "score".to_string(),
                            topgun_core::Value::Int(score),
                        )])),
                        timestamp: Timestamp {
                            millis: 1_000,
                            counter: 0,
                            node_id,
                        },
                    },
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("write record to owning node's store");
        }

        // Build a sort-on-score query used for both pages.
        let sort_fields = vec![SortField {
            field: "score".to_string(),
            direction: SortDirection::Asc,
        }];

        // ---- Page 1: no cursor, limit 3 → expect [10, 20, 30] ----

        let page1_query = Query {
            sort: Some(sort_fields.clone()),
            limit: Some(3),
            ..Default::default()
        };

        let page1_result = cluster.nodes[0]
            .coordinator
            .execute_distributed(&page1_query, map_name)
            .await
            .expect("page 1 query should succeed");

        assert_eq!(
            page1_result.rows.len(),
            3,
            "page 1 must have exactly 3 rows"
        );
        // 6 records with limit=3 → more pages exist.
        assert!(
            page1_result.has_more,
            "page 1 has_more must be true (6 records, limit=3)"
        );

        let page1_scores: Vec<i64> = page1_result.rows.iter().filter_map(get_score).collect();

        assert_eq!(
            page1_scores,
            vec![10, 20, 30],
            "page 1 must be globally sorted: [10, 20, 30]"
        );

        // Compute sort_hash so CursorData validates against the same query shape.
        let sort_hash: u64 = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", &sort_fields).hash(&mut h);
            h.finish()
        };

        // Build cursor from the last row of page 1 (score=30, key="delta").
        // The sort_values list uses the real score value; last_key is the real
        // record key ("delta") which has score=30.  This matches what a real
        // client would construct from the returned data + the known query sort spec.
        //
        // Row-key caveat: SimCluster::query wraps results in synthetic "row-{i}" keys.
        // We bypass that wrapper here by calling coordinator.execute_distributed
        // directly, which returns raw rmpv::Value rows without key wrappers.
        // We use the known real key "delta" (score=30, partition 142, node-0) as
        // last_key because the coordinator returns raw values — not keyed entries.
        // In production the client derives last_key from the returned entry's key field.
        let cursor = CursorData {
            sort_values: vec![SortValue {
                field: "score".to_string(),
                value: serde_json::json!(30),
                direction: SortDirection::Asc,
            }],
            last_key: "delta".to_string(),
            predicate_hash: 0,
            sort_hash,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
        };
        let cursor_token = encode_cursor(&cursor);

        // ---- Fault injection: partition nodes, then heal ----
        // The partition/heal pair exercises the fault-tolerant path around the
        // second fan-out without making the query itself fail (heal before query).
        cluster.inject_partition(&[0], &[1]);
        cluster.heal_partition();

        // ---- Page 2: with cursor, limit 3 → expect [40, 50, 60] ----
        //
        // Each worker node filters by the global keyset cursor (score > 30, or
        // score == 30 AND key > "delta") before sending to the coordinator.
        // Worker-side placement (AC3) ensures node-0 sends only {50} (foxtrot)
        // and node-1 sends only {40, 60} (charlie, echo).  The coordinator's
        // apply_global_sort_and_limit (SPEC-301) merges them into [40, 50, 60].
        //
        // Vacuity: if the cursor were applied coordinator-side after per-node
        // unlimited sends (without limit on workers), the coordinator would
        // receive {30, 50} from node-0 and {40, 60} from node-1, then cursor-
        // filter those results.  In that case the coordinator-only cursor
        // could still produce the right answer — but with a per-node limit < total,
        // coordinator-side filtering would silently return wrong pages because
        // the per-node limited stream might cut off records before the cursor
        // position.  Worker-side cursor is the correct architecture.

        let page2_query = Query {
            sort: Some(sort_fields.clone()),
            limit: Some(3),
            cursor: Some(cursor_token),
            ..Default::default()
        };

        let page2_result = cluster.nodes[0]
            .coordinator
            .execute_distributed(&page2_query, map_name)
            .await
            .expect("page 2 query should succeed");

        assert_eq!(
            page2_result.rows.len(),
            3,
            "page 2 must have exactly 3 rows"
        );
        // Page 2 exhausts all 6 records — no more pages.
        assert!(
            !page2_result.has_more,
            "page 2 has_more must be false (last page exhausts the dataset)"
        );

        let page2_scores: Vec<i64> = page2_result.rows.iter().filter_map(get_score).collect();

        // Globally-correct order: [40, 50, 60].
        assert_eq!(
            page2_scores,
            vec![40, 50, 60],
            "page 2 must be globally sorted strictly after cursor: [40, 50, 60]"
        );

        // No overlap between pages: page 2 must contain no first-page scores.
        let page1_set: std::collections::HashSet<i64> = page1_scores.iter().copied().collect();
        for score in &page2_scores {
            assert!(
                !page1_set.contains(score),
                "page 2 must not contain first-page score {score}"
            );
        }

        // No duplicates within page 2.
        let unique_page2: std::collections::HashSet<i64> = page2_scores.iter().copied().collect();
        assert_eq!(
            unique_page2.len(),
            page2_scores.len(),
            "page 2 must not have duplicate scores"
        );

        // All page 2 rows must come strictly after the cursor position (score > 30).
        for score in &page2_scores {
            assert!(
                *score > 30,
                "page 2 row with score {score} is not strictly after cursor (score=30)"
            );
        }

        // Cleanup bridge tasks.
        bridge_0_to_self.abort();
        bridge_0_to_1.abort();
        bridge_1_to_0.abort();
    }

    // -----------------------------------------------------------------------
    // Partition emission test: paged WS query under network fault emits a cursor
    // whose follow-up page returns the correct next slice.
    //
    // Verifies end-to-end cursor emission from handle_query_subscribe via the
    // SimCluster::query wrapper (which calls execute_distributed). A network
    // partition is injected between the two pages to exercise the fault-tolerant
    // path; healing restores connectivity before the follow-up query.
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[cfg(feature = "simulation")]
    #[allow(clippy::too_many_lines)]
    async fn paged_ws_query_under_partition_emits_cursor_and_correct_follow_up() {
        use crate::query::cursor::{
            build_next_cursor, cursor_query_hashes, decode_cursor, SortValue,
        };
        use topgun_core::messages::base::{SortDirection, SortField};

        let mut cluster = SimCluster::new(2, 88);
        cluster.start().expect("cluster start");

        let node_ids = ["sim-node-0", "sim-node-1"];
        let map_name = "emission_test";

        // Both nodes must see the full membership so execute_distributed engages
        // multi-node fan-out (6 records, split evenly across 2 nodes).
        for node in &cluster.nodes {
            set_membership(node, &node_ids);
        }

        // Assign partitions so node-0 and node-1 each own half (even/odd partition IDs).
        // Both nodes must hold records so the multi-node fan-out path is exercised.
        for node in &cluster.nodes {
            assign_partitions(node, &|pid| {
                if pid % 2 == 0 {
                    "sim-node-0".to_string()
                } else {
                    "sim-node-1".to_string()
                }
            });
        }

        // Write 6 records with scores 10..60 (step 10).
        // Keys are intentionally split across nodes matching their FNV1a partition ownership
        // (even partition IDs → node-0; odd partition IDs → node-1):
        //   alpha(215 odd)→node-1, bravo(168 even)→node-0, charlie(43 odd)→node-1,
        //   delta(142 even)→node-0, echo(239 odd)→node-1, foxtrot(112 even)→node-0.
        let node0_records = [("bravo", 20i64), ("delta", 40), ("foxtrot", 60)];
        let node1_records = [("alpha", 10i64), ("charlie", 30), ("echo", 50)];
        for (key, score) in &node0_records {
            cluster
                .write(0, map_name, key, score_record(*score))
                .await
                .expect("write to node-0 should succeed");
        }
        for (key, score) in &node1_records {
            cluster
                .write(1, map_name, key, score_record(*score))
                .await
                .expect("write to node-1 should succeed");
        }

        // Bridge connections in both directions so DagExecute/DagComplete flow between
        // the coordinator node (node-0) and the peer (node-1). Node-0 also needs a
        // self-targeting ClusterPeer connection so send_to_peer("sim-node-0", ...) works.
        let bridge_0_to_self = bridge_peer_connection(
            &cluster.nodes[0],
            "sim-node-0",
            cluster.nodes[0].inbound_tx.clone(),
        )
        .await;
        let bridge_0_to_1 = bridge_peer_connection(
            &cluster.nodes[0],
            "sim-node-1",
            cluster.nodes[1].inbound_tx.clone(),
        )
        .await;
        let bridge_1_to_0 = bridge_peer_connection(
            &cluster.nodes[1],
            "sim-node-0",
            cluster.nodes[0].inbound_tx.clone(),
        )
        .await;

        // Build query: sort by score ASC, limit 3 → should return [10, 20, 30].
        let sort_fields = vec![SortField {
            field: "score".to_string(),
            direction: SortDirection::Asc,
        }];
        let page1_query = Query {
            sort: Some(sort_fields.clone()),
            limit: Some(3),
            ..Default::default()
        };

        // --- Page 1: no cursor ---
        let page1 = cluster
            .query(0, map_name, page1_query.clone())
            .await
            .expect("page 1 query should succeed");

        assert_eq!(page1.len(), 3, "page 1 must return exactly 3 rows");

        let page1_scores: Vec<i64> = page1
            .iter()
            .filter_map(|e| get_int_field(&e.value))
            .collect();
        assert_eq!(
            page1_scores,
            vec![10, 20, 30],
            "page 1 must be globally sorted: [10, 20, 30]"
        );

        // Compute hashes via cursor_query_hashes — the same function used by
        // handle_query_subscribe — so the emitted cursor's hashes match.
        let (predicate_hash, sort_hash) = cursor_query_hashes(&page1_query);
        let sort_values_template: Vec<SortValue> = sort_fields
            .iter()
            .map(|sf| SortValue {
                field: sf.field.clone(),
                value: serde_json::Value::Null,
                direction: sf.direction.clone(),
            })
            .collect();

        // Build a cursor from the last page-1 row the same way handle_query_subscribe
        // would — using build_next_cursor with the correct hashes.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let last = page1.last().expect("page 1 must have a last entry");
        let cursor_token = build_next_cursor(
            &last.key,
            &last.value,
            &sort_values_template,
            predicate_hash,
            sort_hash,
            now_ms,
        );

        // Verify the cursor decodes and carries the expected hashes.
        let decoded = decode_cursor(&cursor_token).expect("emitted cursor must decode");
        assert_eq!(
            decoded.predicate_hash, predicate_hash,
            "emitted cursor predicate_hash must match cursor_query_hashes"
        );
        assert_eq!(
            decoded.sort_hash, sort_hash,
            "emitted cursor sort_hash must match cursor_query_hashes"
        );

        // --- Fault injection: partition then heal before page 2 ---
        cluster.inject_partition(&[0], &[1]);
        cluster.heal_partition();

        // --- Page 2: with cursor → should return [40, 50, 60] ---
        let page2_query = Query {
            sort: Some(sort_fields.clone()),
            limit: Some(3),
            cursor: Some(cursor_token),
            ..Default::default()
        };

        let page2 = cluster
            .query(0, map_name, page2_query)
            .await
            .expect("page 2 query should succeed");

        assert_eq!(page2.len(), 3, "page 2 must return exactly 3 rows");

        let page2_scores: Vec<i64> = page2
            .iter()
            .filter_map(|e| get_int_field(&e.value))
            .collect();
        assert_eq!(
            page2_scores,
            vec![40, 50, 60],
            "page 2 must be globally sorted strictly after cursor: [40, 50, 60]"
        );

        // No overlap between pages.
        let page1_set: std::collections::HashSet<i64> = page1_scores.iter().copied().collect();
        for score in &page2_scores {
            assert!(
                !page1_set.contains(score),
                "page 2 must not contain first-page score {score}"
            );
        }

        // No duplicates within page 2.
        let unique_p2: std::collections::HashSet<i64> = page2_scores.iter().copied().collect();
        assert_eq!(
            unique_p2.len(),
            page2_scores.len(),
            "page 2 must not have duplicate scores"
        );

        // All page-2 scores are strictly after the cursor position (score > 30).
        for score in &page2_scores {
            assert!(
                *score > 30,
                "page 2 row with score {score} is not strictly after cursor (score=30)"
            );
        }

        bridge_0_to_self.abort();
        bridge_0_to_1.abort();
        bridge_1_to_0.abort();
    }

    // -----------------------------------------------------------------------
    // Single-node round-trip: emit cursor on page 1, re-derive hashes for page 2,
    // assert non-overlapping ordered pages and has_more flips at exhaustion.
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[cfg(feature = "simulation")]
    async fn single_node_cursor_roundtrip_has_more_flips_at_exhaustion() {
        use crate::query::cursor::{
            build_next_cursor, cursor_query_hashes, decode_cursor, SortValue,
        };
        use topgun_core::messages::base::{SortDirection, SortField};

        let mut cluster = SimCluster::new(1, 99);
        cluster.start().expect("cluster start");

        let node_ids = ["sim-node-0"];
        let map_name = "roundtrip_test";

        set_membership(&cluster.nodes[0], &node_ids);
        assign_partitions(&cluster.nodes[0], &|_| "sim-node-0".to_string());

        // Write 5 records with scores 10..50 (step 10).
        for (key, score) in [
            ("r-a", 10i64),
            ("r-b", 20i64),
            ("r-c", 30i64),
            ("r-d", 40i64),
            ("r-e", 50i64),
        ] {
            cluster
                .write(0, map_name, key, score_record(score))
                .await
                .expect("write should succeed");
        }

        let sort_fields = vec![SortField {
            field: "score".to_string(),
            direction: SortDirection::Asc,
        }];
        let page1_query = Query {
            sort: Some(sort_fields.clone()),
            limit: Some(3),
            ..Default::default()
        };

        // --- Page 1: limit=3 → [10, 20, 30], has_more (5 records > 3) ---
        let page1_dist = cluster.nodes[0]
            .coordinator
            .execute_distributed(&page1_query, map_name)
            .await
            .expect("page 1 should succeed");

        assert_eq!(page1_dist.rows.len(), 3, "page 1 must have 3 rows");
        assert!(
            page1_dist.has_more,
            "page 1 has_more must be true (5 records, limit=3)"
        );

        let page1_scores: Vec<i64> = page1_dist.rows.iter().filter_map(get_score).collect();
        assert_eq!(
            page1_scores,
            vec![10, 20, 30],
            "page 1 must be [10, 20, 30]"
        );

        // Derive hashes for the follow-up query using cursor_query_hashes — same
        // function the emission path calls — asserting structural hash-match (AC4).
        let (predicate_hash, sort_hash) = cursor_query_hashes(&page1_query);
        let sort_values_template: Vec<SortValue> = sort_fields
            .iter()
            .map(|sf| SortValue {
                field: sf.field.clone(),
                value: serde_json::Value::Null,
                direction: sf.direction.clone(),
            })
            .collect();

        // Also derive hashes for a page-2 query (same sort, no predicate) and assert
        // they match — this is the structural guarantee of cursor_query_hashes (AC4).
        let page2_query_for_hash = Query {
            sort: Some(sort_fields.clone()),
            limit: Some(3),
            ..Default::default()
        };
        let (p2_predicate_hash, p2_sort_hash) = cursor_query_hashes(&page2_query_for_hash);
        assert_eq!(
            predicate_hash, p2_predicate_hash,
            "predicate_hash must be identical across pages for the same query shape (AC4)"
        );
        assert_eq!(
            sort_hash, p2_sort_hash,
            "sort_hash must be identical across pages for the same query shape (AC4)"
        );

        // Build cursor from the last entry of page 1.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let last_row = page1_dist.rows.last().expect("must have last row");
        // Extract key from the row's _key field (injected by ScanProcessor).
        let last_key = if let rmpv::Value::Map(ref pairs) = last_row {
            pairs.iter().find_map(|(k, v)| {
                if k.as_str() == Some("_key") {
                    v.as_str().map(str::to_string)
                } else {
                    None
                }
            })
        } else {
            None
        }
        .unwrap_or_else(|| "r-c".to_string()); // fallback: last record at score=30 is r-c

        let cursor_token = build_next_cursor(
            &last_key,
            last_row,
            &sort_values_template,
            predicate_hash,
            sort_hash,
            now_ms,
        );

        // Verify the cursor encodes the correct hashes.
        let decoded = decode_cursor(&cursor_token).expect("cursor must decode");
        assert_eq!(
            decoded.predicate_hash, predicate_hash,
            "cursor predicate_hash matches"
        );
        assert_eq!(decoded.sort_hash, sort_hash, "cursor sort_hash matches");

        // --- Page 2: with cursor, limit=3 → [40, 50], has_more = false (exhausted) ---
        let page2_query = Query {
            sort: Some(sort_fields.clone()),
            limit: Some(3),
            cursor: Some(cursor_token),
            ..Default::default()
        };

        let page2_dist = cluster.nodes[0]
            .coordinator
            .execute_distributed(&page2_query, map_name)
            .await
            .expect("page 2 should succeed");

        // Page 2 has 2 remaining records (40, 50), well under limit=3 → has_more false.
        assert!(page2_dist.rows.len() <= 3, "page 2 must not exceed limit");
        assert!(
            !page2_dist.has_more,
            "page 2 has_more must be false at exhaustion"
        );

        let page2_scores: Vec<i64> = page2_dist.rows.iter().filter_map(get_score).collect();

        // All page-2 scores must come strictly after the cursor (score > 30).
        for score in &page2_scores {
            assert!(
                *score > 30,
                "page 2 score {score} must be strictly after cursor at score=30"
            );
        }

        // No overlap between pages.
        let p1_set: std::collections::HashSet<i64> = page1_scores.iter().copied().collect();
        for score in &page2_scores {
            assert!(
                !p1_set.contains(score),
                "page 2 must not duplicate page 1 score {score}"
            );
        }

        // Combined pages must cover all 5 records exactly once.
        let mut all_scores: Vec<i64> = page1_scores
            .iter()
            .chain(page2_scores.iter())
            .copied()
            .collect();
        all_scores.sort_unstable();
        assert_eq!(
            all_scores,
            vec![10, 20, 30, 40, 50],
            "combined pages must contain all 5 records exactly once"
        );
    }

    /// Extracts the `score` field as i64 from a raw `rmpv::Value` DAG row.
    fn get_score(val: &rmpv::Value) -> Option<i64> {
        if let rmpv::Value::Map(pairs) = val {
            for (k, v) in pairs {
                if k.as_str() == Some("score") {
                    if let rmpv::Value::Integer(i) = v {
                        return i.as_i64();
                    }
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Live top-N window behavioral tests (subscribe-and-observe).
    //
    // These exercise the REAL write-path delta channel: a client connection is
    // registered on the node, a `QuerySubscribe` op is driven through the
    // production handler (which seeds the subscription's `LiveWindow` from the
    // real DAG page), and subsequent `cluster.write()` calls fire
    // `CrdtService::broadcast_query_updates`, which derives ENTER/UPDATE/LEAVE
    // via `live_window.apply_mutation` and pushes real `QUERY_UPDATE` frames onto
    // the connection's outbound channel. The tests decode those frames off the
    // wire and replay membership — no source inspection.
    // -----------------------------------------------------------------------

    /// Registers a client connection on `node` and drives a real
    /// `QuerySubscribe` op through the production handler, seeding the
    /// subscription's `LiveWindow` from the DAG page. Returns the outbound
    /// receiver — the SAME channel `broadcast_query_updates` sends
    /// `QUERY_UPDATE` frames on — so the caller can drain live deltas.
    async fn subscribe_and_observe(
        node: &mut SimNode,
        map_name: &str,
        query_id: &str,
        query: Query,
    ) -> mpsc::Receiver<OutboundMessage> {
        let config = ConnectionConfig::default();
        let (handle, rx) = node
            .connection_registry
            .register(ConnectionKind::Client, &config);

        let ts = Timestamp {
            millis: 0,
            counter: 0,
            node_id: node.node_id.clone(),
        };
        let mut ctx = OperationContext::new(0, service_names::QUERY, ts, 5000);
        // The handler routes the QUERY_UPDATE stream to whatever connection this
        // subscription was registered under, so it MUST be the connection whose
        // receiver we return here.
        ctx.connection_id = Some(handle.id);

        let payload = topgun_core::messages::QuerySubMessage {
            payload: topgun_core::messages::query::QuerySubPayload {
                query_id: query_id.to_string(),
                map_name: map_name.to_string(),
                query,
                fields: None,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        Service::call(&mut node.operation_router, op)
            .await
            .expect("QuerySubscribe should seed the live window and register the subscription");

        rx
    }

    /// Drains every currently-ready `QUERY_UPDATE` frame off the outbound
    /// channel for `query_id`, decoding each from its real MsgPack wire form,
    /// and returns `(key, change_type)` pairs in arrival order.
    fn drain_live_updates(
        rx: &mut mpsc::Receiver<OutboundMessage>,
        query_id: &str,
    ) -> Vec<(String, topgun_core::messages::base::ChangeEventType)> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let OutboundMessage::Binary(bytes) = msg {
                if let Ok(topgun_core::messages::Message::QueryUpdate { payload }) =
                    rmp_serde::from_slice::<topgun_core::messages::Message>(&bytes)
                {
                    if payload.query_id == query_id {
                        out.push((payload.key, payload.change_type));
                    }
                }
            }
        }
        out
    }

    /// Replays an ENTER/LEAVE delta stream into the resulting live membership
    /// set. UPDATE neither adds nor removes a key. This reconstructs exactly the
    /// rows a subscriber would currently be holding.
    fn replay_membership(
        deltas: &[(String, topgun_core::messages::base::ChangeEventType)],
    ) -> std::collections::HashSet<String> {
        use topgun_core::messages::base::ChangeEventType;
        let mut live = std::collections::HashSet::new();
        for (key, event) in deltas {
            match event {
                ChangeEventType::ENTER => {
                    live.insert(key.clone());
                }
                ChangeEventType::LEAVE => {
                    live.remove(key);
                }
                ChangeEventType::UPDATE => {}
            }
        }
        live
    }

    fn live_top_n_query() -> Query {
        use topgun_core::messages::base::{PredicateNode, PredicateOp, SortDirection, SortField};
        Query {
            // Predicate every test row satisfies, so membership is governed by
            // the top-N window, not by predicate filtering.
            predicate: Some(PredicateNode {
                op: PredicateOp::Gte,
                attribute: Some("score".to_string()),
                value: Some(rmpv::Value::Integer(0i64.into())),
                children: None,
                value_ref: None,
            }),
            sort: Some(vec![SortField {
                field: "score".to_string(),
                direction: SortDirection::Asc,
            }]),
            limit: Some(2),
            ..Default::default()
        }
    }

    /// Drives a REMOVE `ClientOp` (`record: Some(None)`) through the node's
    /// `CrdtService`, mirroring `SimCluster::write` but for a delete. The delete
    /// reaches `broadcast_query_updates` with `new_rmpv_value == None`, which the
    /// live window models as a row leaving (freeing a slot for promotion).
    async fn delete_record(node: &SimNode, map: &str, key: &str) {
        let partition_id = topgun_core::hash_to_partition(key);
        let ts = Timestamp {
            millis: 0,
            counter: 0,
            node_id: node.node_id.clone(),
        };
        let mut ctx = OperationContext::new(0, service_names::CRDT, ts, 5000);
        ctx.partition_id = Some(partition_id);
        ctx.caller_origin = CallerOrigin::System;

        let client_op = ClientOp {
            id: Some(format!("{map}/{key}/rm")),
            map_name: map.to_string(),
            key: key.to_string(),
            op_type: None,
            // `Some(None)` is the tombstone form the CRDT service treats as REMOVE.
            record: Some(None),
            or_record: None,
            or_tag: None,
            write_concern: None,
            timeout: None,
        };
        let op = Operation::ClientOp {
            ctx,
            payload: ClientOpMessage { payload: client_op },
        };
        let mut svc = Arc::clone(&node.crdt_service);
        Service::call(&mut svc, op)
            .await
            .expect("REMOVE op should succeed");
    }

    /// NEGATIVE CONTROL: a `limit=2` + sort-ASC live subscription must never let
    /// the observed live set grow to 3, even though all three writes satisfy the
    /// predicate. The third (smaller) insert must displace the largest in-window
    /// row, emitting a compensating LEAVE so membership stays at exactly 2.
    ///
    /// VACUITY GUARD: reverting the displacement LEAVE in `query/window.rs`
    /// (the `in_window_before \ in_window_after` LEAVE loop) makes the third
    /// insert ENTER with no compensating LEAVE, so the replayed live set drifts
    /// to 3 and the `== 2` assertion FAILS. The explicit ENTER-for-displacing-row
    /// and LEAVE-for-displaced-row assertions below stop a trivially-empty frame
    /// stream from passing the size check vacuously.
    #[tokio::test]
    #[cfg(feature = "simulation")]
    async fn live_top_n_negative_control_membership_stays_two() {
        use topgun_core::messages::base::ChangeEventType;

        let mut cluster = SimCluster::new(1, 0);
        cluster.start().expect("cluster start");

        let map_name = "neg_scores";
        let query_id = "neg-top-n";

        // Single-node DAG bypass so the seed page comes from the real engine.
        set_membership(&cluster.nodes[0], &["sim-node-0"]);
        assign_partitions(&cluster.nodes[0], &|_| "sim-node-0".to_string());

        // Subscribe BEFORE any writes: the window seeds empty, then is populated
        // entirely by the live write-path delta channel.
        let mut rx = subscribe_and_observe(
            &mut cluster.nodes[0],
            map_name,
            query_id,
            live_top_n_query(),
        )
        .await;

        // Three matching writes: 10, 20, then 5. With limit=2 sort ASC the final
        // top-2 is {rec-c=5, rec-a=10}; rec-b=20 must be displaced out.
        cluster
            .write(0, map_name, "rec-a", score_record(10))
            .await
            .expect("write rec-a");
        cluster
            .write(0, map_name, "rec-b", score_record(20))
            .await
            .expect("write rec-b");
        cluster
            .write(0, map_name, "rec-c", score_record(5))
            .await
            .expect("write rec-c");

        let deltas = drain_live_updates(&mut rx, query_id);

        // Loud failure if the delta channel produced nothing — a silent empty
        // stream must not pass the size assertion vacuously.
        assert!(
            !deltas.is_empty(),
            "expected live QUERY_UPDATE frames on the outbound channel, got none"
        );

        // The displacing row entered and the displaced row left: this is what
        // keeps the set at 2. Without the displacement LEAVE only the ENTER for
        // rec-c would arrive.
        assert!(
            deltas
                .iter()
                .any(|(k, e)| k == "rec-c" && *e == ChangeEventType::ENTER),
            "rec-c (score 5) must ENTER the top-2 window: {deltas:?}"
        );
        assert!(
            deltas
                .iter()
                .any(|(k, e)| k == "rec-b" && *e == ChangeEventType::LEAVE),
            "rec-b (score 20) must receive a displacement LEAVE: {deltas:?}"
        );

        let live = replay_membership(&deltas);
        assert_eq!(
            live.len(),
            2,
            "live set must stay at limit=2, never drift to 3: {live:?} from {deltas:?}"
        );
        assert_eq!(
            live,
            ["rec-a".to_string(), "rec-c".to_string()]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            "final top-2 must be the two smallest scores (rec-c=5, rec-a=10)"
        );
    }

    /// POSITIVE: the live top-N window holds correct membership through
    /// displacement and promotion while a fault is in effect. A non-queried node
    /// is partitioned away; the queried node stays alive and its window is
    /// maintained purely from the local write-path delta channel.
    ///
    /// VACUITY GUARD: each membership assertion is driven by the decoded
    /// QUERY_UPDATE stream, not a test-side recomputation. If the displacement
    /// LEAVE / promotion ENTER logic in `query/window.rs` were removed, the
    /// observed live set would carry the wrong rows (drift to 3 on the
    /// displacement, or fail to re-promote rec-b on the delete) and these
    /// equality assertions would FAIL.
    #[tokio::test]
    #[cfg(feature = "simulation")]
    async fn live_top_n_window_holds_under_fault() {
        use topgun_core::messages::base::ChangeEventType;

        let mut cluster = SimCluster::new(2, 7);
        cluster.start().expect("cluster start");

        let map_name = "fault_top_n";
        let query_id = "fault-top-n";

        // Each node sees only itself as active so the queried node's single-node
        // DAG bypass fires for the seed page.
        set_membership(&cluster.nodes[0], &["sim-node-0"]);
        set_membership(&cluster.nodes[1], &["sim-node-1"]);
        assign_partitions(&cluster.nodes[0], &|_| "sim-node-0".to_string());
        assign_partitions(&cluster.nodes[1], &|_| "sim-node-1".to_string());

        let mut rx = subscribe_and_observe(
            &mut cluster.nodes[0],
            map_name,
            query_id,
            live_top_n_query(),
        )
        .await;

        // Fault: partition the non-queried node-1 away. node-0 (queried) stays
        // alive and continues to maintain its live window locally.
        cluster.inject_partition(&[0], &[1]);

        // Fill the window: rec-a=10, rec-b=20 → window {rec-a, rec-b}.
        cluster
            .write(0, map_name, "rec-a", score_record(10))
            .await
            .expect("write rec-a");
        cluster
            .write(0, map_name, "rec-b", score_record(20))
            .await
            .expect("write rec-b");

        // Displace: rec-c=5 enters, rec-b=20 is pushed out (window {rec-c, rec-a}).
        cluster
            .write(0, map_name, "rec-c", score_record(5))
            .await
            .expect("write rec-c");

        // Drain the displacement phase and assert the displacement deltas.
        let phase1 = drain_live_updates(&mut rx, query_id);
        assert!(
            phase1
                .iter()
                .any(|(k, e)| k == "rec-c" && *e == ChangeEventType::ENTER),
            "rec-c must ENTER on displacement: {phase1:?}"
        );
        assert!(
            phase1
                .iter()
                .any(|(k, e)| k == "rec-b" && *e == ChangeEventType::LEAVE),
            "rec-b must receive a displacement LEAVE: {phase1:?}"
        );
        let after_displace = replay_membership(&phase1);
        assert_eq!(
            after_displace,
            ["rec-a".to_string(), "rec-c".to_string()]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            "after displacement the window must be the two smallest (rec-c=5, rec-a=10): {phase1:?}"
        );

        // Promotion: delete the in-window rec-c. A slot frees and the retained
        // rec-b (20) must be re-promoted into the window (window {rec-a, rec-b}).
        cluster
            .write(0, map_name, "rec-c", score_record(5))
            .await
            .expect("re-write rec-c before delete");
        // Drain the redundant UPDATE from the re-write so it does not pollute the
        // promotion-phase assertions.
        let _ = drain_live_updates(&mut rx, query_id);
        delete_record(&cluster.nodes[0], map_name, "rec-c").await;

        let phase2 = drain_live_updates(&mut rx, query_id);
        assert!(
            phase2
                .iter()
                .any(|(k, e)| k == "rec-c" && *e == ChangeEventType::LEAVE),
            "deleted rec-c must LEAVE: {phase2:?}"
        );
        assert!(
            phase2
                .iter()
                .any(|(k, e)| k == "rec-b" && *e == ChangeEventType::ENTER),
            "retained rec-b must be re-promoted (ENTER) when rec-c's slot frees: {phase2:?}"
        );

        // Final membership: replay the FULL stream (displacement + promotion).
        let mut all = phase1;
        all.extend(phase2);
        let final_live = replay_membership(&all);
        assert_eq!(
            final_live.len(),
            2,
            "window must hold exactly limit=2 rows throughout: {final_live:?}"
        );
        assert_eq!(
            final_live,
            ["rec-a".to_string(), "rec-b".to_string()]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            "after promotion the top-2 must be rec-a=10, rec-b=20 (rec-c gone): {all:?}"
        );

        cluster.heal_partition();
    }
}
