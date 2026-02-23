//! Cluster resilience protocols: split-brain detection, graceful leave,
//! mastership claim, and heartbeat complaint processing.
//!
//! Provides four concrete processor structs (`SplitBrainHandler`,
//! `GracefulLeaveProcessor`, `MastershipClaimProcessor`,
//! `HeartbeatComplaintProcessor`) plus supporting types and the pure
//! `decide_merge()` function for split-brain resolution.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::assignment::{compute_assignment, plan_rebalance};
use super::messages::{
    ClusterMessage, ExplicitSuspicionPayload, MembersUpdatePayload, MergeRequestPayload,
    SplitBrainProbePayload,
};
use super::state::{ClusterChange, ClusterState, MigrationCommand};
use super::traits::FailureDetector;
use super::types::{MemberInfo, MembersView, NodeState};
use crate::network::connection::{ConnectionKind, ConnectionRegistry, OutboundMessage};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Decision produced by `decide_merge()` for split-brain resolution.
///
/// Exactly one side of a detected split-brain will compute `LocalShouldMerge`
/// and send a `MergeRequest` to the other side. The decision is deterministic
/// and deadlock-free: member count wins first, then `join_version`, then
/// lexicographic `master_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitBrainMergeDecision {
    /// This cluster should merge into the remote cluster.
    LocalShouldMerge { remote_master_address: String },
    /// The remote cluster should merge into this cluster.
    RemoteShouldMerge,
    /// Cannot merge (different cluster IDs or incompatible versions).
    CannotMerge,
}

/// Information about a remote cluster extracted from a `SplitBrainProbeResponse`.
///
/// Used as input to `decide_merge()`. Not serialized -- local only.
#[derive(Debug, Clone)]
pub struct RemoteClusterInfo {
    pub cluster_id: String,
    pub master_id: String,
    pub master_address: String,
    pub member_count: u32,
    pub view_version: u64,
    pub master_join_version: u64,
}

/// A single heartbeat complaint record tracking who complained and when.
///
/// Keyed by suspect node ID in the `HeartbeatComplaintProcessor`.
#[derive(Debug, Clone)]
pub struct ComplaintRecord {
    pub complainer_id: String,
    pub received_at_ms: u64,
}

// ---------------------------------------------------------------------------
// decide_merge() — pure function
// ---------------------------------------------------------------------------

/// Determines which side of a split-brain should merge into the other.
///
/// The decision is evaluated in priority order:
/// 1. Mismatched `cluster_id` -> `CannotMerge`
/// 2. Higher active member count wins (that side gets `RemoteShouldMerge`)
/// 3. Equal counts: lower master `join_version` wins
/// 4. Equal counts and `join_version`: lower `master_id` (lexicographic) wins
///
/// This ensures exactly one side sends `MergeRequest`, preventing deadlock.
/// Consistent with `MembersView::master()` ordering which also breaks ties
/// by `node_id` lexicographically.
#[must_use]
pub fn decide_merge(
    local_cluster_id: &str,
    local: &MembersView,
    remote: &RemoteClusterInfo,
) -> SplitBrainMergeDecision {
    // Step 1: cluster ID mismatch
    if local_cluster_id != remote.cluster_id {
        return SplitBrainMergeDecision::CannotMerge;
    }

    let local_count = local.active_members().len();
    let remote_count = remote.member_count as usize;

    // Step 2: member count comparison
    match local_count.cmp(&remote_count) {
        std::cmp::Ordering::Greater => return SplitBrainMergeDecision::RemoteShouldMerge,
        std::cmp::Ordering::Less => {
            return SplitBrainMergeDecision::LocalShouldMerge {
                remote_master_address: remote.master_address.clone(),
            }
        }
        std::cmp::Ordering::Equal => {}
    }

    // Step 3: tie-break by master join_version
    let Some(local_master) = local.master() else {
        // No local master means we cannot meaningfully compare; merge into remote.
        return SplitBrainMergeDecision::LocalShouldMerge {
            remote_master_address: remote.master_address.clone(),
        };
    };

    let local_jv = local_master.join_version;
    let remote_jv = remote.master_join_version;

    match local_jv.cmp(&remote_jv) {
        std::cmp::Ordering::Less => return SplitBrainMergeDecision::RemoteShouldMerge,
        std::cmp::Ordering::Greater => {
            return SplitBrainMergeDecision::LocalShouldMerge {
                remote_master_address: remote.master_address.clone(),
            }
        }
        std::cmp::Ordering::Equal => {}
    }

    // Step 4: secondary tie-break by master_id (lexicographic)
    if local_master.node_id < remote.master_id {
        SplitBrainMergeDecision::RemoteShouldMerge
    } else {
        SplitBrainMergeDecision::LocalShouldMerge {
            remote_master_address: remote.master_address.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// HeartbeatComplaintProcessor
// ---------------------------------------------------------------------------

/// Master-side processor that accumulates heartbeat complaints from non-master
/// nodes and triggers suspicion when the threshold is met.
///
/// Uses `parking_lot::RwLock` because all locking methods (`process_complaint`,
/// `should_suspect`, `cleanup_stale_complaints`) are synchronous.
pub struct HeartbeatComplaintProcessor {
    state: Arc<ClusterState>,
    registry: Arc<ConnectionRegistry>,
    failure_detector: Arc<dyn FailureDetector>,
    complaints: parking_lot::RwLock<HashMap<String, Vec<ComplaintRecord>>>,
}

impl HeartbeatComplaintProcessor {
    /// Creates a new heartbeat complaint processor.
    #[must_use]
    pub fn new(
        state: Arc<ClusterState>,
        registry: Arc<ConnectionRegistry>,
        failure_detector: Arc<dyn FailureDetector>,
    ) -> Self {
        Self {
            state,
            registry,
            failure_detector,
            complaints: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Records a complaint, cleans stale entries, and evaluates the suspicion
    /// threshold. Returns `Some(suspect_id)` if 2+ distinct complainers have
    /// reported the same suspect within the suspicion window AND the master's
    /// own `FailureDetector.is_alive()` does NOT override. Returns `None`
    /// otherwise.
    ///
    /// This is a synchronous function; the caller (message handler) is
    /// responsible for calling `mark_suspect()` when `Some` is returned.
    pub fn process_complaint(
        &self,
        complaint: &super::messages::HeartbeatComplaintPayload,
    ) -> Option<String> {
        let now_ms = current_unix_ms();
        let suspicion_timeout = self.state.config.suspicion_timeout_ms;

        let mut complaints = self.complaints.write();

        // Record the complaint.
        let records = complaints
            .entry(complaint.suspect_id.clone())
            .or_default();
        records.push(ComplaintRecord {
            complainer_id: complaint.complainer_id.clone(),
            received_at_ms: now_ms,
        });

        // Clean stale complaints for this suspect.
        records.retain(|r| now_ms.saturating_sub(r.received_at_ms) < suspicion_timeout);

        // Check threshold: 2+ distinct complainers.
        let distinct_complainers: std::collections::HashSet<&str> =
            records.iter().map(|r| r.complainer_id.as_str()).collect();

        if distinct_complainers.len() < 2 {
            return None;
        }

        // Check master override: if the master's own failure detector
        // still considers the suspect alive, discard the complaints.
        if self
            .failure_detector
            .is_alive(&complaint.suspect_id, now_ms)
        {
            tracing::info!(
                suspect = %complaint.suspect_id,
                "master override: failure detector still sees suspect as alive, discarding complaints"
            );
            return None;
        }

        Some(complaint.suspect_id.clone())
    }

    /// Returns `true` if 2+ distinct complainers have reported the given
    /// suspect within the suspicion timeout window.
    #[must_use]
    pub fn should_suspect(&self, suspect_id: &str) -> bool {
        let now_ms = current_unix_ms();
        let suspicion_timeout = self.state.config.suspicion_timeout_ms;
        let complaints = self.complaints.read();

        let Some(records) = complaints.get(suspect_id) else {
            return false;
        };

        let distinct: std::collections::HashSet<&str> = records
            .iter()
            .filter(|r| now_ms.saturating_sub(r.received_at_ms) < suspicion_timeout)
            .map(|r| r.complainer_id.as_str())
            .collect();

        distinct.len() >= 2
    }

    /// Transitions a suspected node to `Suspect` in `MembersView` and
    /// broadcasts an `ExplicitSuspicion` message to all cluster peers.
    #[allow(clippy::unused_async)] // will use await once networking layer sends async
    pub async fn mark_suspect(&self, suspect_id: &str) {
        let view = self.state.current_view();
        let mut new_members = view.members.clone();

        let mut found = false;
        for member in &mut new_members {
            if member.node_id == suspect_id && member.state == NodeState::Active {
                member.state = NodeState::Suspect;
                found = true;
                break;
            }
        }

        if !found {
            return;
        }

        let new_view = MembersView {
            version: view.version + 1,
            members: new_members,
        };
        self.state.update_view(new_view.clone());

        // Broadcast ExplicitSuspicion to all peers.
        let suspicion_msg = ClusterMessage::ExplicitSuspicion(ExplicitSuspicionPayload {
            suspect_id: suspect_id.to_string(),
            reason: "heartbeat complaints threshold reached".to_string(),
            master_view_version: new_view.version,
        });

        broadcast_cluster_message(&self.registry, &suspicion_msg);

        let _ = self.state.change_sender().send(ClusterChange::MemberUpdated(
            MemberInfo {
                node_id: suspect_id.to_string(),
                host: String::new(),
                client_port: 0,
                cluster_port: 0,
                state: NodeState::Suspect,
                join_version: 0,
            },
        ));
    }

    /// Transitions a `Suspect` node to `Dead` if the suspicion timeout has
    /// expired without heartbeat resumption. Emits `ClusterChange::MemberUpdated`.
    #[allow(clippy::unused_async)] // will use await once networking layer sends async
    pub async fn mark_dead_if_timeout(&self, suspect_id: &str) {
        let now_ms = current_unix_ms();
        let suspicion_timeout = self.state.config.suspicion_timeout_ms;

        // Check if the failure detector has received a recent heartbeat.
        if self.failure_detector.is_alive(suspect_id, now_ms) {
            return;
        }

        let view = self.state.current_view();
        let Some(member) = view.get_member(suspect_id) else {
            return;
        };

        if member.state != NodeState::Suspect {
            return;
        }

        // Check if enough time has passed since the node was marked Suspect.
        // Use the last heartbeat time as a proxy for when suspicion started.
        let last_hb = self.failure_detector.last_heartbeat(suspect_id).unwrap_or(0);
        if now_ms.saturating_sub(last_hb) < suspicion_timeout {
            return;
        }

        let mut new_members = view.members.clone();
        for m in &mut new_members {
            if m.node_id == suspect_id {
                m.state = NodeState::Dead;
                break;
            }
        }

        let new_view = MembersView {
            version: view.version + 1,
            members: new_members,
        };
        self.state.update_view(new_view.clone());

        // Broadcast updated membership.
        let update_msg = ClusterMessage::MembersUpdate(MembersUpdatePayload {
            view: new_view,
            cluster_time_ms: now_ms,
        });
        broadcast_cluster_message(&self.registry, &update_msg);

        let _ = self.state.change_sender().send(ClusterChange::MemberUpdated(
            MemberInfo {
                node_id: suspect_id.to_string(),
                host: String::new(),
                client_port: 0,
                cluster_port: 0,
                state: NodeState::Dead,
                join_version: 0,
            },
        ));
    }

    /// Removes complaints older than `suspicion_timeout_ms`.
    pub fn cleanup_stale_complaints(&self, now_ms: u64) {
        let suspicion_timeout = self.state.config.suspicion_timeout_ms;
        let mut complaints = self.complaints.write();

        complaints.retain(|_, records| {
            records.retain(|r| now_ms.saturating_sub(r.received_at_ms) < suspicion_timeout);
            !records.is_empty()
        });
    }
}

// ---------------------------------------------------------------------------
// GracefulLeaveProcessor
// ---------------------------------------------------------------------------

/// Handles graceful leave requests: marks the node as Leaving, migrates
/// partitions away, then removes the node from the cluster.
pub struct GracefulLeaveProcessor {
    state: Arc<ClusterState>,
    registry: Arc<ConnectionRegistry>,
    migration_tx: mpsc::Sender<MigrationCommand>,
    failure_detector: Arc<dyn FailureDetector>,
}

impl GracefulLeaveProcessor {
    /// Creates a new graceful leave processor.
    #[must_use]
    pub fn new(
        state: Arc<ClusterState>,
        registry: Arc<ConnectionRegistry>,
        migration_tx: mpsc::Sender<MigrationCommand>,
        failure_detector: Arc<dyn FailureDetector>,
    ) -> Self {
        Self {
            state,
            registry,
            migration_tx,
            failure_detector,
        }
    }

    /// Processes a graceful leave request for the given node.
    ///
    /// Only the master processes leave requests. Non-master nodes should
    /// forward the `LeaveRequest` to the current master.
    ///
    /// # Errors
    ///
    /// Returns an error if the node is not found in the current `MembersView`.
    pub async fn process_leave(&self, node_id: &str) -> anyhow::Result<()> {
        // Guard: only the master processes leave requests.
        if !self.state.is_master() {
            // Forward to master (non-master nodes should not call this directly;
            // the message handler routes the LeaveRequest to the master).
            let view = self.state.current_view();
            if let Some(master) = view.master() {
                let leave_msg = ClusterMessage::LeaveRequest(
                    super::messages::LeaveRequestPayload {
                        node_id: node_id.to_string(),
                        reason: Some("forwarded leave request".to_string()),
                    },
                );
                let _ = send_to_peer(&self.registry, &master.node_id, &leave_msg).await;
            }
            return Ok(());
        }

        // Step 2: Mark the node as Leaving.
        let view = self.state.current_view();
        let mut new_members = view.members.clone();
        let mut found = false;
        for member in &mut new_members {
            if member.node_id == node_id {
                member.state = NodeState::Leaving;
                found = true;
                break;
            }
        }

        if !found {
            return Err(anyhow::anyhow!("node {node_id} not found in members view"));
        }

        let new_view = MembersView {
            version: view.version + 1,
            members: new_members,
        };
        self.state.update_view(new_view.clone());

        // Broadcast MembersUpdate.
        let update_msg = ClusterMessage::MembersUpdate(MembersUpdatePayload {
            view: new_view,
            cluster_time_ms: current_unix_ms(),
        });
        broadcast_cluster_message(&self.registry, &update_msg);

        // Step 3: Cancel active migrations involving the leaving node.
        let active_migrations = self.state.active_migrations.read().await;
        let to_cancel: Vec<u32> = active_migrations
            .iter()
            .filter(|(_, m)| m.source == node_id || m.destination == node_id)
            .map(|(&pid, _)| pid)
            .collect();
        drop(active_migrations);

        for pid in to_cancel {
            let _ = self.migration_tx.send(MigrationCommand::Cancel(pid)).await;
        }

        // Step 4: Check partitions owned by the leaving node.
        let owned_partitions = self.state.partition_table.partitions_for_node(node_id);

        if owned_partitions.is_empty() {
            // No partitions to migrate; skip directly to removal.
            self.remove_node(node_id).await;
            return Ok(());
        }

        // Step 5: Compute new assignment excluding the leaving node.
        let current_view = self.state.current_view();
        let active_members: Vec<MemberInfo> = current_view
            .members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .cloned()
            .collect();

        let partition_count = self.state.partition_table.partition_count();
        let backup_count = self.state.config.backup_count;
        let target_assignments = compute_assignment(&active_members, partition_count, backup_count);

        // Step 6: Plan and execute migrations.
        let tasks = plan_rebalance(&self.state.partition_table, &target_assignments);

        for task in tasks {
            let _ = self
                .migration_tx
                .send(MigrationCommand::Start(task))
                .await;
        }

        // Step 7: The caller (or a background monitor) must watch for
        // `ClusterChange::PartitionMoved` events until all partitions are
        // migrated away from the leaving node. When
        // `partition_table.partitions_for_node(node_id)` returns empty,
        // call `remove_node()`.
        //
        // For now, we check immediately in case all partitions were already
        // empty (race-free because we hold the migration_tx).
        let remaining = self.state.partition_table.partitions_for_node(node_id);
        if remaining.is_empty() {
            self.remove_node(node_id).await;
        }

        Ok(())
    }

    /// Removes a node from the cluster after all partitions have been migrated.
    #[allow(clippy::unused_async)] // will use await once networking layer sends async
    async fn remove_node(&self, node_id: &str) {
        let view = self.state.current_view();
        let removed_member = view.get_member(node_id).cloned();

        let new_members: Vec<MemberInfo> = view
            .members
            .iter()
            .filter(|m| m.node_id != node_id)
            .cloned()
            .collect();

        let new_view = MembersView {
            version: view.version + 1,
            members: new_members,
        };
        self.state.update_view(new_view.clone());

        // Broadcast final MembersUpdate.
        let update_msg = ClusterMessage::MembersUpdate(MembersUpdatePayload {
            view: new_view,
            cluster_time_ms: current_unix_ms(),
        });
        broadcast_cluster_message(&self.registry, &update_msg);

        // Emit MemberRemoved event.
        if let Some(member) = removed_member {
            let _ = self
                .state
                .change_sender()
                .send(ClusterChange::MemberRemoved(member));
        }

        // Cleanup: remove from failure detector.
        self.failure_detector.remove(node_id);
    }
}

// ---------------------------------------------------------------------------
// MastershipClaimProcessor
// ---------------------------------------------------------------------------

/// Detects master failure and coordinates mastership claim with majority
/// agreement from reachable peers.
pub struct MastershipClaimProcessor {
    state: Arc<ClusterState>,
    registry: Arc<ConnectionRegistry>,
    failure_detector: Arc<dyn FailureDetector>,
}

impl MastershipClaimProcessor {
    /// Creates a new mastership claim processor.
    #[must_use]
    pub fn new(
        state: Arc<ClusterState>,
        registry: Arc<ConnectionRegistry>,
        failure_detector: Arc<dyn FailureDetector>,
    ) -> Self {
        Self {
            state,
            registry,
            failure_detector,
        }
    }

    /// Returns `false` if the failure detector considers the master dead.
    #[must_use]
    pub fn check_master_alive(&self) -> bool {
        let view = self.state.current_view();
        let Some(master) = view.master() else {
            return false;
        };

        // If we are the master, we are obviously alive.
        if master.node_id == self.state.local_node_id {
            return true;
        }

        let now_ms = current_unix_ms();
        self.failure_detector.is_alive(&master.node_id, now_ms)
    }

    /// Attempts to claim mastership after the current master is detected as dead.
    ///
    /// Returns `true` if this node successfully claimed mastership.
    ///
    /// # Errors
    ///
    /// Returns an error if the claim coordination encounters a fatal failure.
    pub async fn attempt_claim(&self) -> anyhow::Result<bool> {
        let view = self.state.current_view();

        // Identify the dead master.
        let Some(current_master) = view.master() else {
            // No master at all; compute candidate from active members.
            return self.try_claim_as_candidate(&view).await;
        };

        let master_id = current_master.node_id.clone();

        // Verify the master is actually dead.
        let now_ms = current_unix_ms();
        if self.failure_detector.is_alive(&master_id, now_ms) {
            return Ok(false);
        }

        self.try_claim_as_candidate(&view).await
    }

    /// Computes the candidate master and claims if this node is the candidate.
    async fn try_claim_as_candidate(&self, view: &MembersView) -> anyhow::Result<bool> {
        // Compute candidate: oldest active member (excluding the dead master).
        // The oldest member is determined by lowest join_version, tie-broken
        // by lexicographic node_id -- same as MembersView::master().
        let candidate = view
            .members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .min_by(|a, b| {
                a.join_version
                    .cmp(&b.join_version)
                    .then_with(|| a.node_id.cmp(&b.node_id))
            });

        let Some(candidate) = candidate else {
            return Ok(false);
        };

        // Only the candidate proceeds with claiming.
        if candidate.node_id != self.state.local_node_id {
            return Ok(false);
        }

        // Verify majority: count reachable peers that agree on the view.
        let active_count = view.active_members().len();
        let majority = (active_count / 2) + 1;

        // Self counts as 1 agreeing peer.
        let mut agree_count: usize = 1;

        // In a real implementation, we would send the view version to all
        // reachable peers and count agreements. For now, count connected
        // cluster peers as agreeing (simplified majority check).
        for handle in self.registry.connections() {
            if handle.kind != ConnectionKind::ClusterPeer {
                continue;
            }
            let meta = handle.metadata.read().await;
            if meta.peer_node_id.is_some() {
                agree_count += 1;
            }
        }

        if agree_count < majority {
            // Cannot reach majority; back off.
            tracing::warn!(
                agree_count,
                majority,
                "mastership claim: cannot reach majority, backing off"
            );
            return Ok(false);
        }

        // Majority agrees: update the view.
        let mut new_members = view.members.clone();
        for member in &mut new_members {
            // Mark the dead master (any non-Active member that was the master).
            if member.state == NodeState::Active {
                let now_ms = current_unix_ms();
                if !self.failure_detector.is_alive(&member.node_id, now_ms)
                    && member.node_id != self.state.local_node_id
                {
                    // This is likely the dead master.
                    member.state = NodeState::Dead;
                }
            }
        }

        let new_view = MembersView {
            version: view.version + 1,
            members: new_members,
        };
        self.state.update_view(new_view.clone());

        // Broadcast MembersUpdate.
        let update_msg = ClusterMessage::MembersUpdate(MembersUpdatePayload {
            view: new_view,
            cluster_time_ms: current_unix_ms(),
        });
        broadcast_cluster_message(&self.registry, &update_msg);

        // Trigger partition rebalancing for the dead master's partitions.
        let current_view = self.state.current_view();
        let active_members: Vec<MemberInfo> = current_view
            .members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .cloned()
            .collect();

        let partition_count = self.state.partition_table.partition_count();
        let backup_count = self.state.config.backup_count;
        let _assignments = compute_assignment(&active_members, partition_count, backup_count);

        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// SplitBrainHandler
// ---------------------------------------------------------------------------

/// Periodic task on the master node that probes seed addresses for split-brain
/// detection and initiates merge when detected.
pub struct SplitBrainHandler {
    state: Arc<ClusterState>,
    registry: Arc<ConnectionRegistry>,
}

impl SplitBrainHandler {
    /// Creates a new split-brain handler.
    #[must_use]
    pub fn new(state: Arc<ClusterState>, registry: Arc<ConnectionRegistry>) -> Self {
        Self { state, registry }
    }

    /// Runs the periodic split-brain detection loop.
    ///
    /// Uses `tokio::select!` to interleave the sleep interval with the
    /// shutdown signal. Checks `state.is_master()` each iteration.
    pub async fn run(self, mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
        let interval_ms = self.state.config.split_brain_check_interval_ms;
        let interval = tokio::time::Duration::from_millis(interval_ms);

        loop {
            tokio::select! {
                () = tokio::time::sleep(interval) => {
                    if let Some(SplitBrainMergeDecision::LocalShouldMerge { remote_master_address }) = self.check_once().await {
                        self.initiate_merge(&remote_master_address).await;
                    }
                }
                result = shutdown_rx.changed() => {
                    if result.is_ok() && *shutdown_rx.borrow() {
                        tracing::info!("split-brain handler shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// Performs one round of seed probing.
    ///
    /// Returns `None` if this node is not master or if no split-brain is
    /// detected. Passes `self.state.config.cluster_id` to `decide_merge()`.
    #[allow(clippy::unused_async)] // will use await once networking layer provides outbound connections
    pub async fn check_once(&self) -> Option<SplitBrainMergeDecision> {
        // Only the master probes for split-brain.
        if !self.state.is_master() {
            return None;
        }

        let view = self.state.current_view();
        let local_cluster_id = &self.state.config.cluster_id;

        // Collect member addresses to filter out already-known seeds.
        let known_addresses: std::collections::HashSet<String> = view
            .members
            .iter()
            .map(|m| format!("{}:{}", m.host, m.cluster_port))
            .collect();

        // Filter seeds not already in the current MembersView.
        let seeds_to_probe: Vec<&String> = self
            .state
            .config
            .seed_addresses
            .iter()
            .filter(|addr| !known_addresses.contains(addr.as_str()))
            .collect();

        if seeds_to_probe.is_empty() {
            return None;
        }

        // Build probe message.
        let master = view.master()?;
        let _probe = ClusterMessage::SplitBrainProbe(SplitBrainProbePayload {
            sender_cluster_id: local_cluster_id.clone(),
            sender_master_id: master.node_id.clone(),
            #[allow(clippy::cast_possible_truncation)]
            sender_member_count: view.active_members().len() as u32,
            sender_view_version: view.version,
        });

        // In a complete implementation, each seed would be probed via the
        // networking layer (TODO-064). The probe response would be parsed
        // into a RemoteClusterInfo and passed to decide_merge(). For now,
        // the probing infrastructure is a skeleton that will be wired when
        // the networking layer provides temporary outbound connections.
        //
        // When a SplitBrainProbeResponse is received:
        //   let remote = RemoteClusterInfo { ... };
        //   let decision = decide_merge(local_cluster_id, &view, &remote);
        //   return Some(decision);

        None
    }

    /// Sends a `MergeRequest` to the remote master.
    #[allow(clippy::unused_async)] // will use await once networking layer provides outbound connections
    async fn initiate_merge(&self, remote_master_address: &str) {
        let view = self.state.current_view();
        let local_cluster_id = &self.state.config.cluster_id;

        let merge_msg = ClusterMessage::MergeRequest(MergeRequestPayload {
            source_cluster_id: local_cluster_id.clone(),
            source_members: view.members.clone(),
            source_view_version: view.version,
        });

        // Send to the remote master address. In a complete implementation,
        // this would open a temporary outbound connection to the remote
        // master via the networking layer.
        tracing::info!(
            remote = %remote_master_address,
            "initiating merge request to remote master"
        );

        // Attempt to find a connection to the remote master by address.
        let bytes = match rmp_serde::to_vec_named(&merge_msg) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("failed to serialize merge request: {e}");
                return;
            }
        };

        for handle in self.registry.connections() {
            if handle.kind != ConnectionKind::ClusterPeer {
                continue;
            }
            // Best-effort: try to send to any peer that might be the remote master.
            let _ = handle.try_send(OutboundMessage::Binary(bytes.clone()));
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Returns the current Unix timestamp in milliseconds.
#[allow(clippy::cast_possible_truncation)]
fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Sends a cluster message to a specific peer by node ID.
async fn send_to_peer(
    registry: &ConnectionRegistry,
    node_id: &str,
    msg: &ClusterMessage,
) -> anyhow::Result<()> {
    let bytes = rmp_serde::to_vec_named(msg)?;

    for handle in registry.connections() {
        if handle.kind != ConnectionKind::ClusterPeer {
            continue;
        }
        let meta = handle.metadata.read().await;
        if meta.peer_node_id.as_deref() == Some(node_id) {
            drop(meta);
            if handle.try_send(OutboundMessage::Binary(bytes)) {
                return Ok(());
            }
            return Err(anyhow::anyhow!(
                "peer {node_id} channel full or disconnected"
            ));
        }
    }

    Err(anyhow::anyhow!("no connection to peer {node_id}"))
}

/// Broadcasts a cluster message to all connected cluster peers.
fn broadcast_cluster_message(registry: &ConnectionRegistry, msg: &ClusterMessage) {
    match rmp_serde::to_vec_named(msg) {
        Ok(bytes) => registry.broadcast(&bytes, ConnectionKind::ClusterPeer),
        Err(e) => tracing::warn!("failed to serialize cluster message for broadcast: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::types::{ClusterConfig, MemberInfo, NodeState};

    fn make_member(node_id: &str, join_version: u64) -> MemberInfo {
        MemberInfo {
            node_id: node_id.to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            state: NodeState::Active,
            join_version,
        }
    }

    fn make_view(members: Vec<MemberInfo>) -> MembersView {
        MembersView {
            version: 1,
            members,
        }
    }

    fn make_remote(
        cluster_id: &str,
        master_id: &str,
        address: &str,
        count: u32,
        jv: u64,
    ) -> RemoteClusterInfo {
        RemoteClusterInfo {
            cluster_id: cluster_id.to_string(),
            master_id: master_id.to_string(),
            master_address: address.to_string(),
            member_count: count,
            view_version: 1,
            master_join_version: jv,
        }
    }

    // -- decide_merge tests --

    #[test]
    fn decide_merge_cannot_merge_different_cluster_id() {
        let local = make_view(vec![make_member("node-a", 1)]);
        let remote = make_remote("other-cluster", "node-b", "10.0.0.2:9090", 1, 1);

        let result = decide_merge("my-cluster", &local, &remote);
        assert_eq!(result, SplitBrainMergeDecision::CannotMerge);
    }

    #[test]
    fn decide_merge_local_wins_more_members() {
        let local = make_view(vec![
            make_member("node-a", 1),
            make_member("node-b", 2),
            make_member("node-c", 3),
        ]);
        let remote = make_remote("cluster-1", "node-x", "10.0.0.5:9090", 1, 1);

        let result = decide_merge("cluster-1", &local, &remote);
        assert_eq!(result, SplitBrainMergeDecision::RemoteShouldMerge);
    }

    #[test]
    fn decide_merge_remote_wins_more_members() {
        let local = make_view(vec![make_member("node-a", 1)]);
        let remote = make_remote("cluster-1", "node-x", "10.0.0.5:9090", 3, 1);

        let result = decide_merge("cluster-1", &local, &remote);
        assert_eq!(
            result,
            SplitBrainMergeDecision::LocalShouldMerge {
                remote_master_address: "10.0.0.5:9090".to_string()
            }
        );
    }

    #[test]
    fn decide_merge_tiebreak_local_lower_join_version() {
        // Equal count (2), local master has lower join_version -> local wins.
        let local = make_view(vec![make_member("node-a", 1), make_member("node-b", 2)]);
        let remote = make_remote("cluster-1", "node-x", "10.0.0.5:9090", 2, 5);

        let result = decide_merge("cluster-1", &local, &remote);
        assert_eq!(result, SplitBrainMergeDecision::RemoteShouldMerge);
    }

    #[test]
    fn decide_merge_tiebreak_remote_lower_join_version() {
        // Equal count (2), remote master has lower join_version -> remote wins.
        let local = make_view(vec![make_member("node-a", 5), make_member("node-b", 6)]);
        let remote = make_remote("cluster-1", "node-x", "10.0.0.5:9090", 2, 1);

        let result = decide_merge("cluster-1", &local, &remote);
        assert_eq!(
            result,
            SplitBrainMergeDecision::LocalShouldMerge {
                remote_master_address: "10.0.0.5:9090".to_string()
            }
        );
    }

    #[test]
    fn decide_merge_tiebreak_equal_jv_lower_master_id_wins() {
        // Equal count, equal join_version -> tie-break by master_id.
        // Local master = "node-a", remote master = "node-z".
        // "node-a" < "node-z", so local side gets RemoteShouldMerge.
        let local = make_view(vec![make_member("node-a", 1), make_member("node-b", 2)]);
        let remote = make_remote("cluster-1", "node-z", "10.0.0.5:9090", 2, 1);

        let result = decide_merge("cluster-1", &local, &remote);
        assert_eq!(result, SplitBrainMergeDecision::RemoteShouldMerge);
    }

    #[test]
    fn decide_merge_tiebreak_equal_jv_higher_master_id_merges() {
        // Verify from the other perspective: local master = "node-z",
        // remote master = "node-a". "node-z" > "node-a", so local should merge.
        let local = make_view(vec![make_member("node-z", 1), make_member("node-y", 2)]);
        let remote = make_remote("cluster-1", "node-a", "10.0.0.5:9090", 2, 1);

        let result = decide_merge("cluster-1", &local, &remote);
        assert_eq!(
            result,
            SplitBrainMergeDecision::LocalShouldMerge {
                remote_master_address: "10.0.0.5:9090".to_string()
            }
        );
    }

    #[test]
    fn decide_merge_no_local_master_merges_into_remote() {
        // No active members -> no master -> merge into remote.
        let local = MembersView {
            version: 1,
            members: vec![],
        };
        let remote = make_remote("cluster-1", "node-x", "10.0.0.5:9090", 2, 1);

        let result = decide_merge("cluster-1", &local, &remote);
        // With zero local members vs 2 remote, local_count < remote_count.
        assert_eq!(
            result,
            SplitBrainMergeDecision::LocalShouldMerge {
                remote_master_address: "10.0.0.5:9090".to_string()
            }
        );
    }

    // -- HeartbeatComplaintProcessor tests --

    fn make_complaint_processor() -> (HeartbeatComplaintProcessor, Arc<ClusterState>) {
        let config = Arc::new(ClusterConfig {
            suspicion_timeout_ms: 10_000,
            ..ClusterConfig::default()
        });
        let (state, _rx) = ClusterState::new(config, "master-1".to_string());
        let state = Arc::new(state);

        // Set up a view where master-1 is the master.
        state.update_view(MembersView {
            version: 1,
            members: vec![
                MemberInfo {
                    node_id: "master-1".to_string(),
                    host: "127.0.0.1".to_string(),
                    client_port: 8080,
                    cluster_port: 9090,
                    state: NodeState::Active,
                    join_version: 1,
                },
                MemberInfo {
                    node_id: "node-2".to_string(),
                    host: "127.0.0.2".to_string(),
                    client_port: 8080,
                    cluster_port: 9090,
                    state: NodeState::Active,
                    join_version: 2,
                },
                MemberInfo {
                    node_id: "node-3".to_string(),
                    host: "127.0.0.3".to_string(),
                    client_port: 8080,
                    cluster_port: 9090,
                    state: NodeState::Active,
                    join_version: 3,
                },
                MemberInfo {
                    node_id: "suspect-1".to_string(),
                    host: "127.0.0.4".to_string(),
                    client_port: 8080,
                    cluster_port: 9090,
                    state: NodeState::Active,
                    join_version: 4,
                },
            ],
        });

        // Use a DeadlineFailureDetector that considers nodes dead
        // if no heartbeat within 5000ms.
        let fd = Arc::new(
            crate::cluster::failure_detector::DeadlineFailureDetector::new(5000),
        );

        let registry = Arc::new(ConnectionRegistry::new());
        let processor =
            HeartbeatComplaintProcessor::new(Arc::clone(&state), registry, fd);

        (processor, state)
    }

    #[test]
    fn complaint_single_complainer_insufficient() {
        let (processor, _state) = make_complaint_processor();

        let complaint = super::super::messages::HeartbeatComplaintPayload {
            complainer_id: "node-2".to_string(),
            complainer_view_version: 1,
            suspect_id: "suspect-1".to_string(),
            suspect_view_version: 1,
        };

        // Single complaint should return None.
        let result = processor.process_complaint(&complaint);
        assert!(result.is_none(), "single complainer should not trigger suspicion");
    }

    #[test]
    fn complaint_two_distinct_complainers_triggers_suspicion() {
        let (processor, _state) = make_complaint_processor();

        // Record a very old heartbeat for the suspect so the failure detector
        // considers it dead (deadline expired). Without this, the
        // DeadlineFailureDetector defaults to "alive" when no heartbeat exists.
        processor.failure_detector.heartbeat("suspect-1", 0);

        let complaint1 = super::super::messages::HeartbeatComplaintPayload {
            complainer_id: "node-2".to_string(),
            complainer_view_version: 1,
            suspect_id: "suspect-1".to_string(),
            suspect_view_version: 1,
        };

        let complaint2 = super::super::messages::HeartbeatComplaintPayload {
            complainer_id: "node-3".to_string(),
            complainer_view_version: 1,
            suspect_id: "suspect-1".to_string(),
            suspect_view_version: 1,
        };

        // First complaint: not enough.
        let result1 = processor.process_complaint(&complaint1);
        assert!(result1.is_none());

        // Second complaint from different node: threshold met.
        // The failure detector considers suspect-1 dead (old heartbeat expired),
        // so master override will NOT apply.
        let result2 = processor.process_complaint(&complaint2);
        assert_eq!(result2, Some("suspect-1".to_string()));
    }

    #[test]
    fn complaint_same_complainer_twice_insufficient() {
        let (processor, _state) = make_complaint_processor();

        let complaint = super::super::messages::HeartbeatComplaintPayload {
            complainer_id: "node-2".to_string(),
            complainer_view_version: 1,
            suspect_id: "suspect-1".to_string(),
            suspect_view_version: 1,
        };

        // Two complaints from the same node should still return None.
        let _ = processor.process_complaint(&complaint);
        let result = processor.process_complaint(&complaint);
        assert!(result.is_none(), "same complainer twice should not trigger suspicion");
    }

    #[test]
    fn complaint_master_override_returns_none() {
        let (processor, _state) = make_complaint_processor();

        // Record a heartbeat for the suspect so the master's failure detector
        // considers it alive.
        let now_ms = current_unix_ms();
        processor
            .failure_detector
            .heartbeat("suspect-1", now_ms);

        let complaint1 = super::super::messages::HeartbeatComplaintPayload {
            complainer_id: "node-2".to_string(),
            complainer_view_version: 1,
            suspect_id: "suspect-1".to_string(),
            suspect_view_version: 1,
        };

        let complaint2 = super::super::messages::HeartbeatComplaintPayload {
            complainer_id: "node-3".to_string(),
            complainer_view_version: 1,
            suspect_id: "suspect-1".to_string(),
            suspect_view_version: 1,
        };

        let _ = processor.process_complaint(&complaint1);
        let result = processor.process_complaint(&complaint2);
        assert!(
            result.is_none(),
            "master override should suppress suspicion when master has recent heartbeat"
        );
    }

    #[test]
    fn complaint_stale_cleanup() {
        let (processor, _state) = make_complaint_processor();

        // Insert a complaint with a very old timestamp.
        {
            let mut complaints = processor.complaints.write();
            complaints
                .entry("suspect-1".to_string())
                .or_default()
                .push(ComplaintRecord {
                    complainer_id: "node-2".to_string(),
                    received_at_ms: 0, // Very old (epoch).
                });
        }

        // Cleanup should remove the stale complaint.
        let now_ms = current_unix_ms();
        processor.cleanup_stale_complaints(now_ms);

        let complaints = processor.complaints.read();
        assert!(
            complaints.get("suspect-1").is_none(),
            "stale complaint should have been cleaned up"
        );
    }

    // -- GracefulLeaveProcessor tests --

    #[test]
    fn graceful_leave_zero_partitions_removed_immediately() {
        // Use a tokio runtime for the async test.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = Arc::new(ClusterConfig::default());
            let (state, mut change_rx) = ClusterState::new(config, "master-1".to_string());
            let state = Arc::new(state);

            state.update_view(MembersView {
                version: 1,
                members: vec![
                    MemberInfo {
                        node_id: "master-1".to_string(),
                        host: "127.0.0.1".to_string(),
                        client_port: 8080,
                        cluster_port: 9090,
                        state: NodeState::Active,
                        join_version: 1,
                    },
                    MemberInfo {
                        node_id: "node-2".to_string(),
                        host: "127.0.0.2".to_string(),
                        client_port: 8080,
                        cluster_port: 9090,
                        state: NodeState::Active,
                        join_version: 2,
                    },
                ],
            });

            let (migration_tx, _migration_rx) = mpsc::channel(16);
            let fd = Arc::new(
                crate::cluster::failure_detector::DeadlineFailureDetector::new(5000),
            );
            let registry = Arc::new(ConnectionRegistry::new());

            let processor = GracefulLeaveProcessor::new(
                Arc::clone(&state),
                registry,
                migration_tx,
                fd,
            );

            // node-2 has zero partitions, so removal should be immediate.
            processor.process_leave("node-2").await.unwrap();

            let view = state.current_view();
            // node-2 should have been removed from the view.
            assert!(
                view.get_member("node-2").is_none(),
                "node-2 should have been removed from members view"
            );

            // Should have emitted MemberRemoved event.
            // Drain events: first is MemberRemoved (after the MembersUpdate for Leaving).
            // We may get multiple events; look for MemberRemoved.
            let mut found_removed = false;
            while let Ok(event) = change_rx.try_recv() {
                if matches!(event, ClusterChange::MemberRemoved(ref m) if m.node_id == "node-2") {
                    found_removed = true;
                    break;
                }
            }
            assert!(found_removed, "expected MemberRemoved event for node-2");
        });
    }
}
