//! Cluster service traits.
//!
//! Defines the 5 core cluster service contracts: `ClusterService`,
//! `MembershipService`, `ClusterPartitionService`, `MigrationService`,
//! and `FailureDetector`. These are trait-only definitions with no
//! implementations.

use std::sync::Arc;

use async_trait::async_trait;
use topgun_core::messages::cluster::PartitionMapPayload;

use crate::service::registry::ManagedService;
use super::messages::MigrateDataPayload;
use super::state::{ClusterChange, ClusterPartitionTable};
use super::types::{
    ClusterHealth, MemberInfo, MembersView, MigrationTask, PartitionAssignment, PartitionState,
};

// ---------------------------------------------------------------------------
// ClusterService
// ---------------------------------------------------------------------------

/// Top-level cluster service providing membership, partitioning, and health.
///
/// Extends `ManagedService` for lifecycle integration with the `ServiceRegistry`.
#[async_trait]
pub trait ClusterService: ManagedService {
    /// Returns this node's unique identifier.
    fn node_id(&self) -> &str;

    /// Returns `true` if this node is the current cluster master.
    fn is_master(&self) -> bool;

    /// Returns the node ID of the current master, if known.
    fn master_id(&self) -> Option<String>;

    /// Returns the current members view (versioned membership snapshot).
    fn members_view(&self) -> Arc<MembersView>;

    /// Returns a reference to the cluster partition table.
    fn partition_table(&self) -> &ClusterPartitionTable;

    /// Subscribes to cluster state changes via an unbounded channel.
    fn subscribe_changes(&self) -> tokio::sync::mpsc::UnboundedReceiver<ClusterChange>;

    /// Returns the current cluster health summary.
    fn health(&self) -> ClusterHealth;
}

// ---------------------------------------------------------------------------
// MembershipService
// ---------------------------------------------------------------------------

/// Manages cluster membership: join/leave ceremonies and member tracking.
#[async_trait]
pub trait MembershipService: Send + Sync {
    /// Returns the current members view.
    fn current_view(&self) -> Arc<MembersView>;

    /// Looks up a single member by node ID.
    fn get_member(&self, node_id: &str) -> Option<MemberInfo>;

    /// Returns all members in the `Active` state.
    fn active_members(&self) -> Vec<MemberInfo>;

    /// Processes a join request and returns the join response.
    async fn handle_join_request(
        &self,
        request: super::messages::JoinRequestPayload,
    ) -> super::messages::JoinResponsePayload;

    /// Processes a graceful leave request for a node.
    async fn handle_leave_request(&self, node_id: &str) -> anyhow::Result<()>;

    /// Forcefully removes a member (e.g., after failure detection timeout).
    async fn remove_member(&self, node_id: &str) -> anyhow::Result<()>;

    /// Applies an externally received members view update.
    fn apply_members_update(&self, view: MembersView);
}

// ---------------------------------------------------------------------------
// ClusterPartitionService
// ---------------------------------------------------------------------------

/// Extended partition management for the cluster layer.
///
/// Provides partition ownership queries, rebalancing, and assignment updates.
#[async_trait]
pub trait ClusterPartitionService: Send + Sync {
    /// Hashes a key to its owning partition ID.
    fn hash_to_partition(&self, key: &str) -> u32;

    /// Returns the owner node ID for a partition, if assigned.
    fn get_owner(&self, partition_id: u32) -> Option<String>;

    /// Returns `true` if this node owns the given partition.
    fn is_local_owner(&self, partition_id: u32) -> bool;

    /// Returns `true` if this node holds a backup replica of the given partition.
    fn is_local_backup(&self, partition_id: u32) -> bool;

    /// Returns the current state of a partition.
    fn get_state(&self, partition_id: u32) -> PartitionState;

    /// Builds a client-facing partition map from the current membership.
    fn get_partition_map(&self, members: &MembersView) -> PartitionMapPayload;

    /// Returns the current partition table version.
    fn version(&self) -> u64;

    /// Computes a rebalance plan for the given membership, returning migration tasks.
    async fn rebalance(&self, members: &MembersView) -> Vec<MigrationTask>;

    /// Applies partition assignment updates (e.g., after migration completes).
    fn apply_partition_update(&self, assignments: &[PartitionAssignment]);

    /// Returns all partition IDs owned or backed up by a given node.
    fn partitions_for_node(&self, node_id: &str) -> Vec<u32>;
}

// ---------------------------------------------------------------------------
// MigrationService
// ---------------------------------------------------------------------------

/// Manages the lifecycle of partition migrations between nodes.
#[async_trait]
pub trait MigrationService: Send + Sync {
    /// Starts executing a batch of migration tasks.
    async fn start_migrations(&self, tasks: Vec<MigrationTask>) -> anyhow::Result<()>;

    /// Cancels a single in-progress migration by partition ID.
    async fn cancel_migration(&self, partition_id: u32) -> anyhow::Result<()>;

    /// Cancels all in-progress migrations.
    async fn cancel_all(&self) -> anyhow::Result<()>;

    /// Handles the start phase of an incoming migration.
    async fn handle_migrate_start(
        &self,
        partition_id: u32,
        destination: &str,
    ) -> anyhow::Result<()>;

    /// Handles incoming migration data (map state chunks and delta ops).
    async fn handle_migrate_data(&self, data: MigrateDataPayload) -> anyhow::Result<()>;

    /// Handles the ready signal from a migration destination.
    async fn handle_migrate_ready(
        &self,
        partition_id: u32,
        source: &str,
    ) -> anyhow::Result<()>;

    /// Returns `true` if the given partition is currently being migrated.
    fn is_migrating(&self, partition_id: u32) -> bool;
}

// ---------------------------------------------------------------------------
// FailureDetector
// ---------------------------------------------------------------------------

/// Pluggable failure detection (e.g., phi-accrual).
///
/// Tracks heartbeat arrivals and computes suspicion levels. The phi-accrual
/// implementation uses configurable thresholds from `ClusterConfig`.
pub trait FailureDetector: Send + Sync {
    /// Records a heartbeat arrival from a node.
    fn heartbeat(&self, node_id: &str, timestamp_ms: u64);

    /// Returns `true` if the node is considered alive at the given timestamp.
    fn is_alive(&self, node_id: &str, timestamp_ms: u64) -> bool;

    /// Returns the timestamp (ms) of the most recent heartbeat from a node.
    fn last_heartbeat(&self, node_id: &str) -> Option<u64>;

    /// Returns the current suspicion level (phi value) for a node.
    /// Higher values indicate greater suspicion of failure.
    fn suspicion_level(&self, node_id: &str, timestamp_ms: u64) -> f64;

    /// Removes all tracking state for a node.
    fn remove(&self, node_id: &str);

    /// Resets all failure detection state.
    fn reset(&self);
}
