//! Membership change reactor for cluster partition rebalancing.
//!
//! Listens for `ClusterChange` events and recomputes partition assignments
//! whenever a node joins or leaves. Broadcasts updated partition tables to
//! peers and updated partition maps to connected clients.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::network::connection::ConnectionRegistry;

use super::{
    ClusterConfig, ClusterState, MigrationCommand, PeerConnectionMap,
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
