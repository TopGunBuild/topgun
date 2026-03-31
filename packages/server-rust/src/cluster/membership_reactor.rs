//! Membership change reactor for cluster partition rebalancing.
//!
//! Listens for `ClusterChange` events and recomputes partition assignments
//! whenever a node joins or leaves. Broadcasts updated partition tables to
//! peers and updated partition maps to connected clients.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::network::connection::ConnectionRegistry;

use super::{
    ClusterChange, ClusterConfig, ClusterMessage, ClusterState, MigrationCommand,
    PartitionTableUpdatePayload, PeerConnectionMap,
    assignment::compute_assignment,
    migration::broadcast_partition_map,
};

// ---------------------------------------------------------------------------
// MembershipReactor
// ---------------------------------------------------------------------------

/// Reacts to cluster membership changes by recomputing partition ownership
/// and broadcasting updates to peers and clients.
///
/// On `MemberAdded` or `MemberRemoved`, this service calls `compute_assignment()`
/// with the current active members, applies the result to `ClusterPartitionTable`,
/// and fans out the updated table to all cluster peers and connected clients.
///
/// On `MemberRemoved`, it additionally cancels any in-flight migrations
/// involving the dead node via `migration_tx`.
pub struct MembershipReactor {
    /// Shared cluster membership view and partition table.
    pub cluster_state: Arc<ClusterState>,
    /// Map of connected peer send-channels for outbound cluster messages.
    pub peers: Arc<PeerConnectionMap>,
    /// Registry of connected client connections for client-side broadcasts.
    pub connection_registry: Arc<ConnectionRegistry>,
    /// Cluster configuration; used for `backup_count` in `compute_assignment()`.
    pub config: Arc<ClusterConfig>,
    /// Channel for sending migration control commands.
    ///
    /// Matches the bounded `mpsc::Sender<MigrationCommand>` type used by
    /// `ClusterChannels::migration_commands`. Used to cancel active migrations
    /// when a node dies.
    pub migration_tx: mpsc::Sender<MigrationCommand>,
}

impl MembershipReactor {
    /// Runs the membership change reactor loop until the channel closes.
    ///
    /// Processes `ClusterChange` events: recomputes partition assignments on
    /// `MemberAdded`/`MemberRemoved`, broadcasts updated tables, and cancels
    /// migrations involving dead nodes.
    pub async fn run(
        self: Arc<Self>,
        mut change_rx: mpsc::UnboundedReceiver<ClusterChange>,
    ) {
        while let Some(change) = change_rx.recv().await {
            match change {
                ClusterChange::MemberAdded(_) => {
                    self.handle_membership_change();
                }
                ClusterChange::MemberRemoved(dead_member) => {
                    self.handle_membership_change();
                    self.handle_member_removed(&dead_member.node_id).await;
                }
                _ => {
                    // Ignore partition and update events -- not our concern.
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Recomputes partition assignments and broadcasts updated tables.
    ///
    /// Called on both `MemberAdded` and `MemberRemoved` events.
    fn handle_membership_change(&self) {
        let view = self.cluster_state.current_view();

        // Collect members as owned values for assignment computation.
        let active_members = view.members.clone();

        let assignments =
            compute_assignment(&active_members, 271, self.config.backup_count);

        // Atomically update the partition table and increment its version.
        self.cluster_state
            .partition_table
            .apply_assignments(&assignments);

        let table_version = self.cluster_state.partition_table.version();

        debug!(version = table_version, "partition table updated after membership change");

        // Broadcast updated partition table to all peers.
        let update_msg = ClusterMessage::PartitionTableUpdate(PartitionTableUpdatePayload {
            assignments,
            version: table_version,
            completed_migrations: vec![],
        });
        self.peers.broadcast(&update_msg, None);

        // Broadcast updated partition map to all connected clients.
        broadcast_partition_map(
            &self.cluster_state.partition_table,
            &view,
            &self.connection_registry,
        );
    }

    /// Removes the dead node's peer connection and cancels its active migrations.
    ///
    /// Called after `handle_membership_change()` on a `MemberRemoved` event so
    /// the peer is no longer reachable for future broadcasts.
    async fn handle_member_removed(&self, dead_node_id: &str) {
        // Remove the dead node from the peer connection map.
        let _ = self.peers.remove(dead_node_id);
        debug!(node = %dead_node_id, "removed dead peer from connection map");

        // Cancel any active migrations involving the dead node.
        let migrations_guard = self.cluster_state.active_migrations.read().await;
        let affected: Vec<u32> = migrations_guard
            .iter()
            .filter(|(_, m)| m.source == dead_node_id || m.destination == dead_node_id)
            .map(|(&pid, _)| pid)
            .collect();
        drop(migrations_guard);

        for partition_id in affected {
            if let Err(e) = self
                .migration_tx
                .send(MigrationCommand::Cancel(partition_id))
                .await
            {
                warn!(
                    node = %dead_node_id,
                    partition_id,
                    error = %e,
                    "failed to send migration cancel command"
                );
            }
        }
    }
}
