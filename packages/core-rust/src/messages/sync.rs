//! Sync domain message types for LWW and `ORMap` synchronization.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/sync-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::hlc::{LWWRecord, ORMapRecord, Timestamp};

use super::base::{ClientOp, WriteConcern};

// ---------------------------------------------------------------------------
// Client operation messages
// ---------------------------------------------------------------------------

/// A single client operation wrapped in a typed message envelope.
///
/// Maps to `ClientOpMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientOpMessage {
    /// Always `"CLIENT_OP"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The wrapped client operation.
    pub payload: ClientOp,
}

/// Payload for a batch of client operations.
///
/// Maps to the `payload` of `OpBatchMessageSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpBatchPayload {
    /// The batch of operations to apply.
    pub ops: Vec<ClientOp>,
    /// Optional write concern level for the entire batch.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub write_concern: Option<WriteConcern>,
    /// Optional timeout in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timeout: Option<f64>,
}

/// A batch of client operations wrapped in a typed message envelope.
///
/// Maps to `OpBatchMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpBatchMessage {
    /// Always `"OP_BATCH"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The batch payload containing operations.
    pub payload: OpBatchPayload,
}

// ---------------------------------------------------------------------------
// LWW sync messages
// ---------------------------------------------------------------------------

/// Initiates LWW map synchronization.
///
/// Maps to `SyncInitMessageSchema` in `sync-schemas.ts`.
/// FLAT message -- fields are directly on the message, no payload wrapper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInitMessage {
    /// Always `"SYNC_INIT"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Name of the map to synchronize.
    pub map_name: String,
    /// Optional timestamp of last successful sync for delta optimization.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_sync_timestamp: Option<f64>,
}

/// Payload for sync root hash response.
///
/// Maps to the `payload` of `SyncRespRootMessageSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRespRootPayload {
    /// Name of the map being synchronized.
    pub map_name: String,
    /// Root hash of the merkle tree.
    pub root_hash: f64,
    /// Server timestamp at time of response.
    pub timestamp: Timestamp,
}

/// Sync response containing the root hash of the merkle tree.
///
/// Maps to `SyncRespRootMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRespRootMessage {
    /// Always `"SYNC_RESP_ROOT"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The root hash payload.
    pub payload: SyncRespRootPayload,
}

/// Payload for sync bucket hashes response.
///
/// Maps to the `payload` of `SyncRespBucketsMessageSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRespBucketsPayload {
    /// Name of the map being synchronized.
    pub map_name: String,
    /// Merkle tree path to this bucket level.
    pub path: String,
    /// Map of bucket index to bucket hash.
    pub buckets: HashMap<String, f64>,
}

/// Sync response containing bucket hashes at a specific tree level.
///
/// Maps to `SyncRespBucketsMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRespBucketsMessage {
    /// Always `"SYNC_RESP_BUCKETS"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The bucket hashes payload.
    pub payload: SyncRespBucketsPayload,
}

/// A single key-record pair in a sync leaf response.
///
/// Inline type in `SyncRespLeafMessageSchema` records array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncLeafRecord {
    /// The key for this record.
    pub key: String,
    /// The LWW record value.
    pub record: LWWRecord<rmpv::Value>,
}

/// Payload for sync leaf records response.
///
/// Maps to the `payload` of `SyncRespLeafMessageSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRespLeafPayload {
    /// Name of the map being synchronized.
    pub map_name: String,
    /// Merkle tree path to this leaf bucket.
    pub path: String,
    /// The leaf records for this bucket.
    pub records: Vec<SyncLeafRecord>,
}

/// Sync response containing leaf-level records.
///
/// Maps to `SyncRespLeafMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRespLeafMessage {
    /// Always `"SYNC_RESP_LEAF"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The leaf records payload.
    pub payload: SyncRespLeafPayload,
}

/// Payload for merkle bucket request.
///
/// Maps to the `payload` of `MerkleReqBucketMessageSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MerkleReqBucketPayload {
    /// Name of the map to query.
    pub map_name: String,
    /// Merkle tree path to the requested bucket.
    pub path: String,
}

/// Request for merkle bucket hashes at a specific path.
///
/// Maps to `MerkleReqBucketMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MerkleReqBucketMessage {
    /// Always `"MERKLE_REQ_BUCKET"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The bucket request payload.
    pub payload: MerkleReqBucketPayload,
}

// ---------------------------------------------------------------------------
// ORMap shared types
// ---------------------------------------------------------------------------

/// A single entry in an `ORMap` sync message containing records and tombstones.
///
/// Maps to `ORMapEntrySchema` in `sync-schemas.ts`.
/// Used across `ORMap` leaf responses, diff responses, and push diffs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapEntry {
    /// The key for this entry.
    pub key: String,
    /// Active records for this key.
    pub records: Vec<ORMapRecord<rmpv::Value>>,
    /// Tombstone tags identifying removed records.
    pub tombstones: Vec<String>,
}

// ---------------------------------------------------------------------------
// ORMap sync messages
// ---------------------------------------------------------------------------

/// Initiates `ORMap` synchronization.
///
/// Maps to `ORMapSyncInitSchema` in `sync-schemas.ts`.
/// FLAT message -- fields are directly on the message, no payload wrapper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapSyncInit {
    /// Always `"ORMAP_SYNC_INIT"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Name of the `ORMap` to synchronize.
    pub map_name: String,
    /// Root hash of the client's merkle tree.
    pub root_hash: f64,
    /// Map of bucket index to bucket hash for delta detection.
    pub bucket_hashes: HashMap<String, f64>,
    /// Optional timestamp of last successful sync for delta optimization.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_sync_timestamp: Option<f64>,
}

/// Payload for `ORMap` sync root hash response.
///
/// Maps to the `payload` of `ORMapSyncRespRootSchema` in `sync-schemas.ts`.
/// Same shape as `SyncRespRootPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapSyncRespRootPayload {
    /// Name of the `ORMap` being synchronized.
    pub map_name: String,
    /// Root hash of the merkle tree.
    pub root_hash: f64,
    /// Server timestamp at time of response.
    pub timestamp: Timestamp,
}

/// `ORMap` sync response containing the root hash.
///
/// Maps to `ORMapSyncRespRootSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapSyncRespRoot {
    /// Always `"ORMAP_SYNC_RESP_ROOT"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The root hash payload.
    pub payload: ORMapSyncRespRootPayload,
}

/// Payload for `ORMap` sync bucket hashes response.
///
/// Maps to the `payload` of `ORMapSyncRespBucketsSchema` in `sync-schemas.ts`.
/// Same shape as `SyncRespBucketsPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapSyncRespBucketsPayload {
    /// Name of the `ORMap` being synchronized.
    pub map_name: String,
    /// Merkle tree path to this bucket level.
    pub path: String,
    /// Map of bucket index to bucket hash.
    pub buckets: HashMap<String, f64>,
}

/// `ORMap` sync response containing bucket hashes.
///
/// Maps to `ORMapSyncRespBucketsSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapSyncRespBuckets {
    /// Always `"ORMAP_SYNC_RESP_BUCKETS"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The bucket hashes payload.
    pub payload: ORMapSyncRespBucketsPayload,
}

/// Payload for `ORMap` merkle bucket request.
///
/// Maps to the `payload` of `ORMapMerkleReqBucketSchema` in `sync-schemas.ts`.
/// Same shape as `MerkleReqBucketPayload`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapMerkleReqBucketPayload {
    /// Name of the `ORMap` to query.
    pub map_name: String,
    /// Merkle tree path to the requested bucket.
    pub path: String,
}

/// `ORMap` merkle bucket request.
///
/// Maps to `ORMapMerkleReqBucketSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapMerkleReqBucket {
    /// Always `"ORMAP_MERKLE_REQ_BUCKET"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The bucket request payload.
    pub payload: ORMapMerkleReqBucketPayload,
}

/// Payload for `ORMap` sync leaf response.
///
/// Maps to the `payload` of `ORMapSyncRespLeafSchema` in `sync-schemas.ts`.
/// Unlike LWW leaves, uses `ORMapEntry` instead of `SyncLeafRecord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapSyncRespLeafPayload {
    /// Name of the `ORMap` being synchronized.
    pub map_name: String,
    /// Merkle tree path to this leaf bucket.
    pub path: String,
    /// The leaf entries for this bucket.
    pub entries: Vec<ORMapEntry>,
}

/// `ORMap` sync response containing leaf-level entries.
///
/// Maps to `ORMapSyncRespLeafSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapSyncRespLeaf {
    /// Always `"ORMAP_SYNC_RESP_LEAF"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The leaf entries payload.
    pub payload: ORMapSyncRespLeafPayload,
}

/// Payload for `ORMap` diff request.
///
/// Maps to the `payload` of `ORMapDiffRequestSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapDiffRequestPayload {
    /// Name of the `ORMap` to query.
    pub map_name: String,
    /// Keys to request diffs for.
    pub keys: Vec<String>,
}

/// `ORMap` diff request for specific keys.
///
/// Maps to `ORMapDiffRequestSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapDiffRequest {
    /// Always `"ORMAP_DIFF_REQUEST"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The diff request payload.
    pub payload: ORMapDiffRequestPayload,
}

/// Payload for `ORMap` diff response.
///
/// Maps to the `payload` of `ORMapDiffResponseSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapDiffResponsePayload {
    /// Name of the `ORMap` being diffed.
    pub map_name: String,
    /// The diff entries.
    pub entries: Vec<ORMapEntry>,
}

/// `ORMap` diff response with entries for requested keys.
///
/// Maps to `ORMapDiffResponseSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapDiffResponse {
    /// Always `"ORMAP_DIFF_RESPONSE"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The diff response payload.
    pub payload: ORMapDiffResponsePayload,
}

/// Payload for `ORMap` push diff.
///
/// Maps to the `payload` of `ORMapPushDiffSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapPushDiffPayload {
    /// Name of the `ORMap` being updated.
    pub map_name: String,
    /// The diff entries to push.
    pub entries: Vec<ORMapEntry>,
}

/// `ORMap` push diff message sent to propagate changes.
///
/// Maps to `ORMapPushDiffSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORMapPushDiff {
    /// Always `"ORMAP_PUSH_DIFF"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The push diff payload.
    pub payload: ORMapPushDiffPayload,
}

// ---------------------------------------------------------------------------
// Write concern response messages
// ---------------------------------------------------------------------------

/// Result of a single operation within a batch acknowledgement.
///
/// Maps to `OpResultSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    /// Identifier of the operation this result refers to.
    pub op_id: String,
    /// Whether the operation succeeded.
    pub success: bool,
    /// The write concern level actually achieved.
    pub achieved_level: WriteConcern,
    /// Optional error message if the operation failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Payload for an operation acknowledgement message.
///
/// Maps to the `payload` of `OpAckMessageSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpAckPayload {
    /// Identifier of the last operation in the acknowledged batch.
    pub last_id: String,
    /// Optional achieved write concern level for the batch.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub achieved_level: Option<WriteConcern>,
    /// Optional per-operation results within the batch.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub results: Option<Vec<OpResult>>,
}

/// Operation acknowledgement message.
///
/// Maps to `OpAckMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpAckMessage {
    /// Always `"OP_ACK"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The acknowledgement payload.
    pub payload: OpAckPayload,
}

/// Payload for an operation rejection message.
///
/// Maps to the `payload` of `OpRejectedMessageSchema` in `sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpRejectedPayload {
    /// Identifier of the rejected operation.
    pub op_id: String,
    /// Human-readable reason for the rejection.
    pub reason: String,
    /// Optional machine-readable error code.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub code: Option<f64>,
}

/// Operation rejection message.
///
/// Maps to `OpRejectedMessageSchema` in `sync-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpRejectedMessage {
    /// Always `"OP_REJECTED"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The rejection payload.
    pub payload: OpRejectedPayload,
}

// ---------------------------------------------------------------------------
// Batched messages
// ---------------------------------------------------------------------------

/// A batch of messages packed into a single binary frame.
///
/// Maps to `BatchMessageSchema` in `sync-schemas.ts`.
/// FLAT message -- fields are directly on the message, no payload wrapper.
/// The `data` field carries length-prefixed binary messages as `Vec<u8>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchMessage {
    /// Always `"BATCH"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Number of individual messages in the batch.
    pub count: f64,
    /// Binary payload containing length-prefixed messages.
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}
