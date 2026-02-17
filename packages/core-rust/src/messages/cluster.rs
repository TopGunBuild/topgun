//! Cluster domain payload structs for partition map distribution, inter-node
//! subscription forwarding, and distributed search coordination.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/cluster-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Status of a node within the cluster.
///
/// Maps to the inline `z.enum(...)` in `NodeInfoSchema.status` in
/// `cluster-schemas.ts`. Variant names use `SCREAMING_CASE` to match
/// TS wire values directly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum NodeStatus {
    ACTIVE,
    JOINING,
    LEAVING,
    SUSPECTED,
    FAILED,
}

// ---------------------------------------------------------------------------
// Partition Map types
// ---------------------------------------------------------------------------

/// Network endpoints for a cluster node.
///
/// Maps to the inline `endpoints` object in `NodeInfoSchema` in
/// `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeEndpoints {
    /// WebSocket endpoint URL.
    pub websocket: String,

    /// Optional HTTP endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub http: Option<String>,
}

/// Information about a single node in the cluster.
///
/// Maps to `NodeInfoSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    /// Unique identifier for this node.
    pub node_id: String,

    /// Network endpoints for reaching this node.
    pub endpoints: NodeEndpoints,

    /// Current membership status.
    pub status: NodeStatus,
}

/// Ownership information for a single partition.
///
/// Maps to `PartitionInfoSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionInfo {
    /// Partition identifier (0-based).
    pub partition_id: u32,

    /// Node ID of the partition owner.
    pub owner_node_id: String,

    /// Node IDs holding backup replicas.
    pub backup_node_ids: Vec<String>,
}

/// Full partition map describing cluster topology.
///
/// Maps to `PartitionMapPayloadSchema` in `cluster-schemas.ts`.
/// Distributed to clients so they can route operations directly to
/// the owning node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionMapPayload {
    /// Monotonically increasing version for optimistic staleness detection.
    pub version: u32,

    /// Total number of partitions in the cluster (typically 271).
    pub partition_count: u32,

    /// All known cluster nodes and their endpoints.
    pub nodes: Vec<NodeInfo>,

    /// Assignment of partitions to nodes.
    pub partitions: Vec<PartitionInfo>,

    /// Timestamp (ms since epoch) when this map was generated.
    pub generated_at: i64,
}

/// Payload for requesting the current partition map.
///
/// Maps to the `payload` of `PartitionMapRequestSchema` in `cluster-schemas.ts`.
/// Includes the client's current version for delta comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionMapRequestPayload {
    /// Client's current partition map version, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_version: Option<u32>,
}
