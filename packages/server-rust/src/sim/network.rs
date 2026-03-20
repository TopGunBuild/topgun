//! Simulation network layer with fault injection and inter-node transport.
//!
//! Provides [`SimTransport`] for routing operations between [`SimNode`](super::cluster::SimNode)
//! instances without `WebSockets`, and [`SimNetwork`] for injecting network faults
//! (partitions, delays, reordering) in simulation tests.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tower::Service;

use topgun_core::messages::OpBatchMessage;
use topgun_core::Timestamp;

use crate::service::domain::CrdtService;
use crate::service::operation::{service_names, Operation, OperationContext};

// ---------------------------------------------------------------------------
// SimTransport
// ---------------------------------------------------------------------------

/// Routes broadcast operations between `SimNode` instances without `WebSockets`.
///
/// Each `SimNode` holds a clone of `SimTransport`. When a node broadcasts
/// an op-batch, it calls [`SimTransport::deliver()`], which forwards the
/// batch to all other registered nodes via the Tower `Service<Operation>`
/// interface on `Arc<CrdtService>`.
///
/// Delivery is filtered by the shared `SimNetwork` partition state: if the
/// source–target pair is partitioned, the message is silently dropped.
#[derive(Clone)]
pub struct SimTransport {
    /// Shared registry of all nodes' `CrdtService` handles in this cluster.
    peers: Arc<RwLock<HashMap<String, Arc<CrdtService>>>>,
    /// Shared network fault-injection layer consulted before each delivery.
    network: Arc<SimNetwork>,
}

impl SimTransport {
    /// Creates an empty transport backed by the given network fault layer.
    #[must_use]
    pub fn new(network: Arc<SimNetwork>) -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            network,
        }
    }

    /// Registers a node's `CrdtService` handle for inter-node message delivery.
    pub fn register(&self, node_id: &str, svc: Arc<CrdtService>) {
        self.peers.write().insert(node_id.to_string(), svc);
    }

    /// Removes a node's `CrdtService` handle (used when killing a node).
    pub fn unregister(&self, node_id: &str) {
        self.peers.write().remove(node_id);
    }

    /// Delivers an `OpBatch` to all registered peers except the sender.
    ///
    /// Constructs `Operation::OpBatch` with `connection_id: None` so that
    /// `handle_op_batch` skips client auth/validation (same pattern used
    /// for internal/system calls).
    ///
    /// Peers that are partitioned from `from_node` (per `SimNetwork`) are
    /// silently skipped — the message is dropped for that link only.
    ///
    /// # Errors
    ///
    /// Returns an error if constructing or dispatching an operation fails
    /// for all peers. Individual peer errors are silently ignored.
    pub async fn deliver(&self, from_node: &str, batch: OpBatchMessage) -> anyhow::Result<()> {
        // Snapshot peers under the read lock, then release before async calls.
        let targets: Vec<(String, Arc<CrdtService>)> = {
            let peers = self.peers.read();
            peers
                .iter()
                .filter(|(id, _)| id.as_str() != from_node)
                .map(|(id, svc)| (id.clone(), Arc::clone(svc)))
                .collect()
        };

        for (target_id, svc) in targets {
            // Respect partition state: drop message silently for partitioned links.
            if self.network.is_partitioned(from_node, &target_id) {
                continue;
            }

            let ts = Timestamp {
                millis: 0,
                counter: 0,
                node_id: from_node.to_string(),
            };
            let ctx = OperationContext::new(0, service_names::CRDT, ts, 5000);
            // connection_id is already None from OperationContext::new()
            let op = Operation::OpBatch {
                ctx,
                payload: batch.clone(),
            };
            let mut svc_clone = svc;
            // Ignore errors from individual peers (they may be down).
            let _ = Service::call(&mut svc_clone, op).await;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SimNetwork
// ---------------------------------------------------------------------------

/// Network fault injection layer for simulation tests.
///
/// Tracks partitions, delays, and reordering state between nodes.
/// These methods are structural only -- they update internal state but
/// do not yet affect actual message delivery. A future spec will wire
/// `SimTransport::deliver()` to consult this state.
pub struct SimNetwork {
    /// Set of partitioned node pairs (bidirectional).
    partitions: RwLock<HashSet<(String, String)>>,
    /// Per-link delays.
    delays: RwLock<HashMap<(String, String), Duration>>,
    /// Per-link reordering flags.
    reorder_flags: RwLock<HashSet<(String, String)>>,
}

impl SimNetwork {
    /// Creates a new `SimNetwork` with no faults injected.
    #[must_use]
    pub fn new() -> Self {
        Self {
            partitions: RwLock::new(HashSet::new()),
            delays: RwLock::new(HashMap::new()),
            reorder_flags: RwLock::new(HashSet::new()),
        }
    }

    /// Injects a network partition between two groups of nodes.
    ///
    /// After this call, messages between any node in `nodes_a` and any node
    /// in `nodes_b` are blocked (bidirectional). Takes node IDs as strings.
    pub fn inject_partition(&self, nodes_a: &[String], nodes_b: &[String]) {
        let mut parts = self.partitions.write();
        for a in nodes_a {
            for b in nodes_b {
                parts.insert((a.clone(), b.clone()));
                parts.insert((b.clone(), a.clone()));
            }
        }
    }

    /// Heals all network partitions, restoring full connectivity.
    pub fn heal_partition(&self) {
        self.partitions.write().clear();
    }

    /// Adds a delay to messages between two specific nodes (bidirectional).
    pub fn delay(&self, node_a: &str, node_b: &str, duration: Duration) {
        let mut delays = self.delays.write();
        delays.insert((node_a.to_string(), node_b.to_string()), duration);
        delays.insert((node_b.to_string(), node_a.to_string()), duration);
    }

    /// Enables message reordering between two specific nodes (bidirectional).
    pub fn reorder(&self, node_a: &str, node_b: &str) {
        let mut flags = self.reorder_flags.write();
        flags.insert((node_a.to_string(), node_b.to_string()));
        flags.insert((node_b.to_string(), node_a.to_string()));
    }

    /// Returns true if a partition exists between the two nodes.
    #[must_use]
    pub fn is_partitioned(&self, from: &str, to: &str) -> bool {
        self.partitions
            .read()
            .contains(&(from.to_string(), to.to_string()))
    }

    /// Returns the delay for a link, if any.
    #[must_use]
    pub fn get_delay(&self, from: &str, to: &str) -> Option<Duration> {
        self.delays
            .read()
            .get(&(from.to_string(), to.to_string()))
            .copied()
    }

    /// Returns true if reordering is enabled for the link.
    #[must_use]
    pub fn is_reordered(&self, from: &str, to: &str) -> bool {
        self.reorder_flags
            .read()
            .contains(&(from.to_string(), to.to_string()))
    }
}

impl Default for SimNetwork {
    fn default() -> Self {
        Self::new()
    }
}
