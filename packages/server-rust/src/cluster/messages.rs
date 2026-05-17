//! Cluster inter-node wire messages.
//!
//! These messages are exchanged between cluster nodes over dedicated inter-node
//! connections, separate from the client `Message` enum. They use the same
//! `MsgPack` serialization format (`rmp_serde::to_vec_named()`).

use serde::{Deserialize, Serialize};

use super::types::{MemberInfo, MembersView, PartitionAssignment};

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// Discriminator for CRDT map types in migration data chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapType {
    Lww,
    Or,
}

/// A chunk of serialized map state transferred during migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapStateChunk {
    pub map_name: String,
    pub data: Vec<u8>,
    pub map_type: MapType,
}

/// A single delta operation transferred during migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaOp {
    pub map_name: String,
    pub key: String,
    pub entry: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Master-election types
// ---------------------------------------------------------------------------

/// Machine-readable category for join rejections.
///
/// Allows the joiner to distinguish retry-able from permanent rejection
/// without parsing the human-readable `reject_reason` string. Older peers
/// that omit `reject_code` are treated as `NotMasterYet` (retry-able via
/// the master-election wait).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JoinRejectReason {
    /// Receiver is not (yet) the master; joiner should wait for the
    /// elected master's `MasterElected` broadcast and re-target.
    NotMasterYet,
    /// Receiver rejected the join due to authentication failure.
    AuthFailed,
    /// Receiver does not support the joiner's `protocol_version`.
    ProtocolVersionMismatch,
    /// Joiner's `cluster_id` does not match the receiver's.
    WrongClusterId,
    /// Receiver's cluster has reached its configured member cap.
    ClusterFull,
}

/// Broadcast by the elected master immediately after `self_promote_as_master`.
///
/// Recipients in `WaitForMasterElection` state use this to re-target their
/// join attempt; recipients that are already members ignore it (their
/// `MembershipReactor` does not interpret this variant — election happens
/// at the `ClusterMessage` layer, not the `ClusterChange` layer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MasterElectedPayload {
    /// `host:cluster_port` of the elected master, dial-able by other nodes.
    pub master_address: String,
    /// `node_id` of the elected master (lexicographically lowest in the formation).
    pub master_node_id: String,
    /// Monotonic election term, incremented on each new election. Initialized
    /// to 1 on first election; reserved for future master-failover elections
    /// (v1 always sets term = 1).
    pub term: u64,
    /// Unique identifier for this election round (UUID v4 string). Recipients
    /// use this to deduplicate identical broadcasts that may arrive via multiple
    /// held peer connections.
    pub election_id: String,
}

// ---------------------------------------------------------------------------
// ClusterMessage enum (19 variants)
// ---------------------------------------------------------------------------

/// Top-level cluster protocol message.
///
/// Internally tagged on `"type"` with `SCREAMING_SNAKE_CASE` variant names.
/// Covers membership (4), heartbeat (3), partition (2), migration (5),
/// split-brain (3), forwarding (1), and DAG execution (3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClusterMessage {
    // -- Membership (5) ----------------------------------------------------
    JoinRequest(JoinRequestPayload),
    JoinResponse(JoinResponsePayload),
    MembersUpdate(MembersUpdatePayload),
    LeaveRequest(LeaveRequestPayload),
    /// Broadcast by the elected master after winning deterministic tiebreak.
    /// Nodes in `WaitForMasterElection` state re-target their join attempt;
    /// already-joined nodes ignore it (`MembershipReactor` does not observe it).
    MasterElected(MasterElectedPayload),

    // -- Heartbeat (3) -----------------------------------------------------
    Heartbeat(HeartbeatPayload),
    HeartbeatComplaint(HeartbeatComplaintPayload),
    ExplicitSuspicion(ExplicitSuspicionPayload),

    // -- Partition (2) -----------------------------------------------------
    PartitionTableUpdate(PartitionTableUpdatePayload),
    FetchPartitionTable,

    // -- Migration (5) -----------------------------------------------------
    MigrateStart(MigrateStartPayload),
    MigrateData(MigrateDataPayload),
    MigrateReady(MigrateReadyPayload),
    MigrateFinalize(MigrateFinalizePayload),
    MigrateCancel(MigrateCancelPayload),

    // -- Split-Brain (3) ---------------------------------------------------
    SplitBrainProbe(SplitBrainProbePayload),
    SplitBrainProbeResponse(SplitBrainProbeResponsePayload),
    MergeRequest(MergeRequestPayload),

    // -- Forwarding (1) ----------------------------------------------------
    OpForward(OpForwardPayload),

    // -- DAG execution (3) -------------------------------------------------
    /// Distributed DAG execution plan sent to participating nodes.
    DagExecute(DagExecutePayload),
    /// Batch of data items flowing between nodes during DAG execution.
    DagData(DagDataPayload),
    /// Signals completion or error of a DAG execution on a node.
    DagComplete(DagCompletePayload),
}

// ---------------------------------------------------------------------------
// Membership payloads
// ---------------------------------------------------------------------------

/// Payload for a node requesting to join the cluster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequestPayload {
    pub node_id: String,
    pub host: String,
    pub client_port: u16,
    pub cluster_port: u16,
    pub cluster_id: String,
    pub protocol_version: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub auth_token: Option<String>,
}

/// Payload for the master's response to a join request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinResponsePayload {
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reject_reason: Option<String>,
    /// Machine-readable rejection category. `None` on accepted responses and
    /// on responses from older peers that predate this field; treated as
    /// `NotMasterYet` by the state machine (safe retry-able default).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reject_code: Option<JoinRejectReason>,
    /// The responding node's own `node_id`. Populated on `NotMasterYet`
    /// rejections so the joiner can build the deterministic-tiebreak set
    /// without relying on an (empty) `members_view`. `None` on older peers
    /// that predate this field; joiner falls back to self-only tiebreak set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub responder_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub members_view: Option<MembersView>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub partition_assignments: Option<Vec<PartitionAssignment>>,
}

/// Payload broadcasting an updated members view to all nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembersUpdatePayload {
    pub view: MembersView,
    pub cluster_time_ms: u64,
}

/// Payload for a node requesting to leave the cluster gracefully.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaveRequestPayload {
    pub node_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Heartbeat payloads
// ---------------------------------------------------------------------------

/// Periodic heartbeat sent between cluster nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatPayload {
    pub sender_id: String,
    pub timestamp_ms: u64,
    pub members_view_version: u64,
    pub suspected_nodes: Vec<String>,
}

/// Complaint from a node that suspects another node has failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatComplaintPayload {
    pub complainer_id: String,
    pub complainer_view_version: u64,
    pub suspect_id: String,
    pub suspect_view_version: u64,
}

/// Master-originated explicit suspicion declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplicitSuspicionPayload {
    pub suspect_id: String,
    pub reason: String,
    pub master_view_version: u64,
}

// ---------------------------------------------------------------------------
// Partition payloads
// ---------------------------------------------------------------------------

/// Payload broadcasting updated partition assignments to all nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionTableUpdatePayload {
    pub assignments: Vec<PartitionAssignment>,
    pub version: u64,
    pub completed_migrations: Vec<String>,
}

// FetchPartitionTable is a unit variant -- no payload struct needed.

// ---------------------------------------------------------------------------
// Migration payloads
// ---------------------------------------------------------------------------

/// Initiates a partition migration to a destination node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateStartPayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub destination_node_id: String,
}

/// Carries partition data (map state and delta ops) during migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateDataPayload {
    pub partition_id: u32,
    pub map_states: Vec<MapStateChunk>,
    pub delta_ops: Vec<DeltaOp>,
    pub source_version: u32,
}

/// Signals that the destination has received all migration data and is ready.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateReadyPayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub source_node_id: String,
}

/// Finalizes a migration, transferring ownership to the new owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateFinalizePayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub new_owner: String,
}

/// Cancels an in-progress migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateCancelPayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Split-Brain payloads
// ---------------------------------------------------------------------------

/// Probe sent to detect split-brain scenarios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitBrainProbePayload {
    pub sender_cluster_id: String,
    pub sender_master_id: String,
    pub sender_member_count: u32,
    pub sender_view_version: u64,
}

/// Response to a split-brain probe with local cluster state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitBrainProbeResponsePayload {
    pub responder_cluster_id: String,
    pub responder_master_id: String,
    pub responder_member_count: u32,
    pub responder_view_version: u64,
    pub responder_master_join_version: u64,
}

/// Request to merge a minority partition back into the majority cluster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequestPayload {
    pub source_cluster_id: String,
    pub source_members: Vec<MemberInfo>,
    pub source_view_version: u64,
}

// ---------------------------------------------------------------------------
// Forwarding payloads
// ---------------------------------------------------------------------------

/// Forwards a client operation to the partition owner node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpForwardPayload {
    pub source_node_id: String,
    pub target_partition_id: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub client_id: Option<String>,
    /// MsgPack-serialized client `Message`.
    pub payload: Vec<u8>,
}

// ---------------------------------------------------------------------------
// DAG execution payloads
// ---------------------------------------------------------------------------

/// Distributed DAG execution plan sent to participating nodes.
///
/// `plan` is a MsgPack-serialized `ExecutionPlan` (which contains
/// `DagPlanDescriptor`). Receiving nodes deserialize and reconstruct
/// a runtime `Dag` via `Dag::from_descriptor()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DagExecutePayload {
    pub execution_id: String,
    /// MsgPack-serialized `ExecutionPlan`.
    pub plan: Vec<u8>,
}

/// Batch of data items flowing between nodes during DAG execution.
///
/// `items` is a MsgPack-serialized `Vec<rmpv::Value>`, produced by
/// `rmp_serde::to_vec_named()` in `NetworkSenderProcessor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DagDataPayload {
    pub execution_id: String,
    pub source_vertex: String,
    pub dest_vertex: String,
    /// MsgPack-serialized `Vec<rmpv::Value>`.
    pub items: Vec<u8>,
}

/// Signals completion or error of a DAG execution on a node.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DagCompletePayload {
    pub execution_id: String,
    pub node_id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub results: Option<Vec<u8>>,
}
