//! Cluster domain types: enums, structs, and configuration.
//!
//! These types define the internal cluster protocol's data model. They are
//! separate from the client-facing types in `topgun_core::messages::cluster`
//! and use Rust-idiomatic naming conventions.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Internal cluster FSM state for a node.
///
/// This exists alongside `NodeStatus` in core-rust (`topgun_core::messages::cluster`)
/// because they serve different purposes:
/// - `NodeStatus` is the **client-facing** wire type with SCREAMING_CASE variants
///   (ACTIVE, JOINING, LEAVING, SUSPECTED, FAILED) to match the TypeScript SDK.
/// - `NodeState` is the **internal cluster** FSM state with Rust-idiomatic naming
///   and two additional lifecycle variants (`Dead`, `Removed`) that clients never see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeState {
    Joining,
    Active,
    Suspect,
    Leaving,
    Dead,
    Removed,
}

/// State of a partition on a specific node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PartitionState {
    Unassigned,
    Active,
    Migrating,
    Receiving,
    Draining,
    Lost,
}

/// Phase of an active migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationPhase {
    Replicating,
    Ready,
    Finalizing,
    Failed,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// Information about a single cluster member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfo {
    pub node_id: String,
    pub host: String,
    pub client_port: u16,
    pub cluster_port: u16,
    pub state: NodeState,
    pub join_version: u64,
}

/// Versioned snapshot of cluster membership.
///
/// Contains all known members and a monotonically increasing version number
/// that advances on every membership change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembersView {
    pub version: u64,
    pub members: Vec<MemberInfo>,
}

impl MembersView {
    /// Returns the current master: the Active member with the lowest `join_version`.
    /// Ties are broken by lexicographic `node_id`. Returns `None` for empty views
    /// or views with no Active members.
    pub fn master(&self) -> Option<&MemberInfo> {
        self.members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .min_by(|a, b| {
                a.join_version
                    .cmp(&b.join_version)
                    .then_with(|| a.node_id.cmp(&b.node_id))
            })
    }

    /// Returns `true` only if the given `node_id` matches the computed master.
    pub fn is_master(&self, node_id: &str) -> bool {
        self.master()
            .map_or(false, |master| master.node_id == node_id)
    }

    /// Returns all members with `state == NodeState::Active`.
    pub fn active_members(&self) -> Vec<&MemberInfo> {
        self.members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .collect()
    }

    /// Finds a member by `node_id`.
    pub fn get_member(&self, node_id: &str) -> Option<&MemberInfo> {
        self.members.iter().find(|m| m.node_id == node_id)
    }
}

/// Metadata for a single partition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionMeta {
    pub partition_id: u32,
    pub owner: String,
    pub backups: Vec<String>,
    pub state: PartitionState,
    pub version: u32,
}

/// Target assignment for a partition (output of the assignment algorithm).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionAssignment {
    pub partition_id: u32,
    pub owner: String,
    pub backups: Vec<String>,
}

/// A single partition migration to execute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationTask {
    pub partition_id: u32,
    pub source: String,
    pub destination: String,
    pub new_backups: Vec<String>,
}

/// Tracking state for an in-progress migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMigration {
    pub migration_id: String,
    pub partition_id: u32,
    pub source: String,
    pub destination: String,
    pub state: MigrationPhase,
    pub started_at_ms: u64,
}

/// Summary of cluster health for diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterHealth {
    pub node_count: usize,
    pub active_nodes: usize,
    pub suspect_nodes: usize,
    pub partition_table_version: u64,
    pub active_migrations: usize,
    pub is_master: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub master_node_id: Option<String>,
}

/// Configuration for cluster behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterConfig {
    pub cluster_id: String,
    pub seed_addresses: Vec<String>,
    pub heartbeat_interval_ms: u64,
    pub phi_threshold: f64,
    pub max_sample_size: usize,
    pub min_std_dev_ms: u64,
    pub max_no_heartbeat_ms: u64,
    pub suspicion_timeout_ms: u64,
    pub backup_count: u32,
    pub max_parallel_migrations: u32,
    pub split_brain_check_interval_ms: u64,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            cluster_id: String::new(),
            seed_addresses: Vec::new(),
            heartbeat_interval_ms: 1000,
            phi_threshold: 8.0,
            max_sample_size: 200,
            min_std_dev_ms: 100,
            max_no_heartbeat_ms: 5000,
            suspicion_timeout_ms: 10_000,
            backup_count: 1,
            max_parallel_migrations: 2,
            split_brain_check_interval_ms: 30_000,
        }
    }
}
