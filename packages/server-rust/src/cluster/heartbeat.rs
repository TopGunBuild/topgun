//! Heartbeat service for cluster failure detection.
//!
//! Periodically broadcasts `Heartbeat` messages to all peers, monitors
//! liveness via `PhiAccrualFailureDetector`, and drives the Suspect -> Dead
//! state transitions on the master node.

use std::sync::Arc;

use dashmap::DashMap;

use super::{
    ClusterConfig, ClusterState, PeerConnectionMap, PhiAccrualFailureDetector,
};

// ---------------------------------------------------------------------------
// HeartbeatService
// ---------------------------------------------------------------------------

/// Periodically sends heartbeats to peers and drives failure detection.
///
/// On the master node, this service transitions unresponsive peers through
/// `Suspect` -> `Dead` states using `PhiAccrualFailureDetector` phi scores and
/// a configurable `suspicion_timeout_ms` window. Non-master nodes forward
/// complaints to the master instead of acting unilaterally.
pub struct HeartbeatService {
    /// Shared cluster membership view and partition table.
    pub cluster_state: Arc<ClusterState>,
    /// Map of connected peer send-channels for outbound cluster messages.
    pub peers: Arc<PeerConnectionMap>,
    /// Statistical failure detector tracking per-node heartbeat intervals.
    pub failure_detector: Arc<PhiAccrualFailureDetector>,
    /// Cluster configuration (heartbeat_interval_ms, suspicion_timeout_ms, etc.).
    pub config: Arc<ClusterConfig>,
    /// Timestamp (ms) when each node was first marked Suspect.
    ///
    /// Used to enforce `suspicion_timeout_ms`: a node must remain suspected for
    /// at least this duration before being promoted to Dead.
    pub suspected_at: DashMap<String, u64>,
}
