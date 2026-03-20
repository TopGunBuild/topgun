//! Simulation cluster harness for deterministic testing.
//!
//! Provides [`SimCluster`] (N-node orchestrator) and [`SimNode`] (single node
//! with full service stack backed by in-memory storage). All types are behind
//! `#[cfg(feature = "simulation")]`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tower::Service;

use topgun_core::messages::sync::{ClientOpMessage, OpBatchMessage, OpBatchPayload};
use topgun_core::{ClientOp, HLC, LWWRecord, ORMapRecord, SystemClock, Timestamp};

use crate::cluster::state::ClusterState;
use crate::cluster::types::ClusterConfig;
use crate::network::connection::ConnectionRegistry;
use crate::service::domain::query::QueryRegistry;
use crate::service::domain::search::SearchRegistry;
use crate::service::domain::{
    CoordinationService, CrdtService, MessagingService, PersistenceService, QueryService,
    SchemaService, SearchService, SyncService,
};
use crate::service::operation::{service_names, CallerOrigin, Operation, OperationContext};
use crate::service::router::OperationRouter;
use crate::service::security::{SecurityConfig, WriteValidator};
use crate::storage::datastores::NullDataStore;
use crate::storage::factory::{ObserverFactory, RecordStoreFactory};
use crate::storage::impls::StorageConfig;
use crate::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use crate::storage::record::RecordValue;

use super::network::{SimNetwork, SimTransport};

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
    pub fn build(
        node_id: impl Into<String>,
        _seed: u64,
        transport: SimTransport,
    ) -> anyhow::Result<Self> {
        let node_id = node_id.into();

        let hlc = Arc::new(Mutex::new(HLC::new(
            node_id.clone(),
            Box::new(SystemClock),
        )));
        let write_validator = Arc::new(WriteValidator::new(
            Arc::new(SecurityConfig::default()),
            hlc,
        ));

        let cluster_config = Arc::new(ClusterConfig::default());
        let (cluster_state, _rx) = ClusterState::new(cluster_config, node_id.clone());
        let cluster_state = Arc::new(cluster_state);
        let connection_registry = Arc::new(ConnectionRegistry::new());

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

        // Build all 7 domain services.
        let crdt_service = Arc::new(CrdtService::new(
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
            write_validator,
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));

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
        router.register(
            service_names::SEARCH,
            Arc::new(SearchService::new(
                Arc::new(SearchRegistry::new()),
                Arc::new(parking_lot::RwLock::new(HashMap::new())),
                Arc::clone(&record_store_factory),
                Arc::clone(&connection_registry),
                search_needs_population,
            )),
        );

        router.register(
            service_names::PERSISTENCE,
            Arc::new(PersistenceService::new(
                connection_registry,
                node_id.clone(),
            )),
        );

        Ok(SimNode {
            node_id,
            crdt_service,
            record_store_factory,
            operation_router: router,
            cluster_state,
            transport,
            alive: true,
        })
    }

    /// Returns whether this node is currently alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Marks this node as dead (simulates crash).
    pub fn kill(&mut self) {
        self.alive = false;
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
        topgun_core::Value::Array(arr) => rmpv::Value::Array(arr.iter().map(value_to_rmpv).collect()),
        topgun_core::Value::Map(map) => {
            let entries: Vec<(rmpv::Value, rmpv::Value)> = map
                .iter()
                .map(|(k, val)| (rmpv::Value::String(k.as_str().into()), value_to_rmpv(val)))
                .collect();
            rmpv::Value::Map(entries)
        }
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
            self.transport.register(&node_id, Arc::clone(&node.crdt_service));
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
        self.transport.register(&node_id, Arc::clone(&node.crdt_service));
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
        let node = self.nodes.get(node_idx)
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
        let node = self.nodes.get(node_idx)
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
                let client_op = match rec.value {
                    RecordValue::Lww { value, timestamp } => {
                        let lww = LWWRecord {
                            value: Some(value_to_rmpv(&value)),
                            timestamp,
                            ttl_ms: None,
                        };
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
                        }
                    }
                    RecordValue::OrMap { records } => {
                        // Deliver each OR-Map entry as a separate op.
                        // Use the first entry's tag as representative for simplicity;
                        // full OR-Map convergence goes through or_write/merkle_sync_pair.
                        if records.is_empty() {
                            continue;
                        }
                        let entry = &records[0];
                        let or_rec = ORMapRecord {
                            value: value_to_rmpv(&entry.value),
                            timestamp: entry.timestamp.clone(),
                            tag: entry.tag.clone(),
                            ttl_ms: None,
                        };
                        ClientOp {
                            id: Some(format!("{map}/{key}/{}", entry.tag)),
                            map_name: map.to_string(),
                            key: key.to_string(),
                            op_type: None,
                            record: None,
                            or_record: Some(Some(or_rec)),
                            or_tag: Some(Some(entry.tag.clone())),
                            write_concern: None,
                            timeout: None,
                        }
                    }
                    RecordValue::OrTombstones { .. } => continue,
                };
                entries.push((node.node_id.clone(), client_op));
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
        let src = self.nodes.get(src_idx)
            .ok_or_else(|| anyhow::anyhow!("src node index {src_idx} out of range"))?;
        let dst = self.nodes.get(dst_idx)
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
                RecordValue::Lww { value: v, timestamp } => {
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
                RecordValue::OrMap { records } => {
                    // Transfer all OR-Map entries; destination merges via CRDT semantics.
                    for entry in records {
                        let op = ClientOp {
                            id: Some(format!("{map}/{key}/{}", entry.tag)),
                            map_name: map.to_string(),
                            key: key.clone(),
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
                        };
                        ops.push(op);
                    }
                    continue;
                }
                RecordValue::OrTombstones { .. } => continue,
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
        self.transport.register(&node_id, Arc::clone(&node.crdt_service));
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
        let node = self.nodes.get(node_idx)
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
                        lhs,
                        rhs,
                        "convergence failure for map={map:?} key={key:?}: \
                         node {first_idx} and node {idx} hold different values",
                    );
                }
            }
        }

        Ok(first.and_then(|(_, v)| v))
    }
}
