//! Heartbeat service for cluster failure detection.
//!
//! Periodically broadcasts `Heartbeat` messages to all peers, monitors
//! liveness via `PhiAccrualFailureDetector`, and drives the Suspect -> Dead
//! state transitions on the master node.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

use super::{
    ClusterChange, ClusterConfig, ClusterMessage, ClusterState, HeartbeatComplaintPayload,
    HeartbeatPayload, NodeState, PeerConnectionMap, PhiAccrualFailureDetector,
};
use super::traits::FailureDetector;

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
    /// Cluster configuration (`heartbeat_interval_ms`, `suspicion_timeout_ms`, etc.).
    pub config: Arc<ClusterConfig>,
    /// Timestamp (ms) when each node was first marked Suspect.
    ///
    /// Used to enforce `suspicion_timeout_ms`: a node must remain suspected for
    /// at least this duration before being promoted to Dead.
    pub suspected_at: DashMap<String, u64>,
}

impl HeartbeatService {
    /// Runs the heartbeat loop until shutdown is signalled.
    ///
    /// Broadcasts a `Heartbeat` message to all peers on every `heartbeat_interval_ms`
    /// tick. After each broadcast, evaluates liveness of all connected peers and
    /// drives `Suspect` -> `Dead` state transitions (master) or forwards complaints
    /// to the master (non-master). Also drains `inbound_rx` to record received
    /// heartbeats and process complaints.
    pub async fn run(
        self: Arc<Self>,
        mut inbound_rx: mpsc::UnboundedReceiver<ClusterMessage>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let interval_ms = self.config.heartbeat_interval_ms;
        let mut ticker =
            tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    self.tick_heartbeat();
                }
                msg = inbound_rx.recv() => {
                    match msg {
                        Some(ClusterMessage::Heartbeat(payload)) => {
                            self.failure_detector.heartbeat(
                                &payload.sender_id,
                                payload.timestamp_ms,
                            );
                            debug!(
                                sender = %payload.sender_id,
                                "received heartbeat"
                            );
                        }
                        Some(ClusterMessage::HeartbeatComplaint(ref payload)) => {
                            self.handle_complaint(payload);
                        }
                        Some(_) => {
                            // Ignore non-heartbeat messages on this channel.
                        }
                        None => {
                            // Inbound channel closed; stop the loop.
                            break;
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Broadcasts a heartbeat to all peers and checks peer liveness.
    fn tick_heartbeat(&self) {
        let now_ms = now_ms();
        let view = self.cluster_state.current_view();

        // Build the list of suspected nodes to include in the broadcast.
        let suspected_nodes: Vec<String> = self
            .peers
            .connected_peers()
            .into_iter()
            .filter(|id| !self.failure_detector.is_alive(id, now_ms))
            .collect();

        let msg = ClusterMessage::Heartbeat(HeartbeatPayload {
            sender_id: self.cluster_state.local_node_id.clone(),
            timestamp_ms: now_ms,
            members_view_version: view.version,
            suspected_nodes,
        });

        self.peers.broadcast(&msg, None);

        // Evaluate liveness of all known peers.
        for peer_id in self.peers.connected_peers() {
            if !self.failure_detector.is_alive(&peer_id, now_ms) {
                if self.cluster_state.is_master() {
                    self.handle_suspect_or_dead(&peer_id, now_ms);
                } else {
                    // Non-master: forward a complaint to the master.
                    let master_id = view.master().map(|m| m.node_id.clone());
                    if let Some(master_id) = master_id {
                        let complaint = ClusterMessage::HeartbeatComplaint(
                            HeartbeatComplaintPayload {
                                complainer_id: self.cluster_state.local_node_id.clone(),
                                complainer_view_version: view.version,
                                suspect_id: peer_id.clone(),
                                suspect_view_version: view
                                    .get_member(&peer_id)
                                    .map_or(0, |m| m.join_version),
                            },
                        );
                        if let Err(e) = self.peers.send_to(&master_id, &complaint) {
                            warn!(
                                master = %master_id,
                                suspect = %peer_id,
                                error = %e,
                                "failed to forward complaint to master"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Handles an inbound `HeartbeatComplaint` (master only).
    ///
    /// If the failure detector also considers the suspected node not alive,
    /// the master marks it as `Suspect`. Complaints only trigger the initial
    /// suspicion — Dead promotion is handled by the heartbeat tick loop.
    fn handle_complaint(&self, payload: &HeartbeatComplaintPayload) {
        if !self.cluster_state.is_master() {
            return;
        }

        let now_ms = now_ms();
        if !self.failure_detector.is_alive(&payload.suspect_id, now_ms) {
            let already_suspected = self.suspected_at.contains_key(&*payload.suspect_id);
            if !already_suspected {
                self.suspected_at
                    .insert(payload.suspect_id.clone(), now_ms);
                self.apply_node_state_update(
                    &payload.suspect_id,
                    NodeState::Suspect,
                    true,
                );
                debug!(node = %payload.suspect_id, "node marked Suspect via complaint");
            }
        }
    }

    /// Drives Suspect / Dead transitions for `node_id` on the master.
    ///
    /// First offence: marks the node `Suspect` and records the suspicion time.
    /// After `suspicion_timeout_ms` elapses: promotes to `Dead` and removes the
    /// node from suspicion tracking.
    fn handle_suspect_or_dead(&self, node_id: &str, now_ms: u64) {
        let already_suspected = self.suspected_at.contains_key(node_id);

        if !already_suspected {
            // First time detected -- mark Suspect.
            self.suspected_at.insert(node_id.to_string(), now_ms);
            self.apply_node_state_update(node_id, NodeState::Suspect, true);
            debug!(node = %node_id, "node marked Suspect");
            return;
        }

        let suspected_since = *self.suspected_at.get(node_id).unwrap();
        if now_ms.saturating_sub(suspected_since) >= self.config.suspicion_timeout_ms {
            // Suspicion window elapsed -- promote to Dead.
            self.suspected_at.remove(node_id);
            self.apply_node_state_update(node_id, NodeState::Dead, false);
            debug!(node = %node_id, "node promoted to Dead");
        }
    }

    /// Applies a state update for one member using the full view update sequence:
    /// load view -> clone members -> set state -> increment version -> store -> emit event.
    ///
    /// `is_suspect` controls which `ClusterChange` variant to emit:
    /// `true` -> `MemberUpdated`, `false` -> `MemberRemoved`.
    fn apply_node_state_update(
        &self,
        node_id: &str,
        new_state: NodeState,
        is_suspect: bool,
    ) {
        // 1. Load current view.
        let current = self.cluster_state.current_view();
        // 2. Clone members vec.
        let mut members = current.members.clone();
        // 3. Find and update target member's state.
        let Some(member_pos) = members.iter().position(|m| m.node_id == node_id) else {
            return;
        };
        members[member_pos].state = new_state;
        let updated_member = members[member_pos].clone();
        // 4. Increment view version.
        let new_version = current.version + 1;
        // 5. Store via update_view().
        let new_view = super::MembersView {
            version: new_version,
            members,
        };
        self.cluster_state.update_view(new_view);
        // 6. Emit ClusterChange.
        let change = if is_suspect {
            ClusterChange::MemberUpdated(updated_member)
        } else {
            ClusterChange::MemberRemoved(updated_member)
        };
        if let Err(e) = self.cluster_state.change_sender().send(change) {
            warn!(node = %node_id, error = %e, "failed to emit cluster change");
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Returns the current wall-clock time as milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
