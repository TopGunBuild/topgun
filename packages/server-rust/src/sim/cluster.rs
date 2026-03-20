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

use topgun_core::messages::sync::ClientOpMessage;
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

        Ok(first.map(|(_, v)| v).unwrap_or(None))
    }
}
