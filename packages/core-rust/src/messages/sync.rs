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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::Timestamp;
    use crate::messages::base::WriteConcern;

    /// Helper: round-trip a value through named `MsgPack` serialization.
    fn roundtrip_named<T>(val: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let bytes = rmp_serde::to_vec_named(val).expect("serialize");
        rmp_serde::from_slice(&bytes).expect("deserialize")
    }

    // ---- Client operation messages ----

    #[test]
    fn client_op_message_roundtrip() {
        let msg = ClientOpMessage {
            r#type: "CLIENT_OP".to_string(),
            payload: ClientOp {
                id: Some("op-1".to_string()),
                map_name: "users".to_string(),
                key: "user-1".to_string(),
                op_type: Some("set".to_string()),
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: Some(WriteConcern::APPLIED),
                timeout: None,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn op_batch_message_roundtrip() {
        let msg = OpBatchMessage {
            r#type: "OP_BATCH".to_string(),
            payload: OpBatchPayload {
                ops: vec![
                    ClientOp {
                        id: Some("op-1".to_string()),
                        map_name: "events".to_string(),
                        key: "evt-1".to_string(),
                        op_type: None,
                        record: None,
                        or_record: None,
                        or_tag: None,
                        write_concern: None,
                        timeout: None,
                    },
                ],
                write_concern: Some(WriteConcern::PERSISTED),
                timeout: Some(5000.0),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- LWW sync messages ----

    #[test]
    fn sync_init_message_roundtrip() {
        let msg = SyncInitMessage {
            r#type: "SYNC_INIT".to_string(),
            map_name: "users".to_string(),
            last_sync_timestamp: Some(1_700_000_000_000.0),
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn sync_init_message_without_timestamp_roundtrip() {
        let msg = SyncInitMessage {
            r#type: "SYNC_INIT".to_string(),
            map_name: "events".to_string(),
            last_sync_timestamp: None,
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn sync_resp_root_message_roundtrip() {
        let msg = SyncRespRootMessage {
            r#type: "SYNC_RESP_ROOT".to_string(),
            payload: SyncRespRootPayload {
                map_name: "users".to_string(),
                root_hash: 12345.0,
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 1,
                    node_id: "node-1".to_string(),
                },
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn sync_resp_buckets_message_roundtrip() {
        let mut buckets = HashMap::new();
        buckets.insert("0".to_string(), 111.0);
        buckets.insert("1".to_string(), 222.0);
        buckets.insert("2".to_string(), 333.0);

        let msg = SyncRespBucketsMessage {
            r#type: "SYNC_RESP_BUCKETS".to_string(),
            payload: SyncRespBucketsPayload {
                map_name: "users".to_string(),
                path: "0".to_string(),
                buckets,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn sync_resp_leaf_message_roundtrip() {
        let msg = SyncRespLeafMessage {
            r#type: "SYNC_RESP_LEAF".to_string(),
            payload: SyncRespLeafPayload {
                map_name: "users".to_string(),
                path: "0/1".to_string(),
                records: vec![
                    SyncLeafRecord {
                        key: "user-1".to_string(),
                        record: LWWRecord {
                            value: Some(rmpv::Value::String("Alice".into())),
                            timestamp: Timestamp {
                                millis: 1_700_000_000_000,
                                counter: 1,
                                node_id: "node-1".to_string(),
                            },
                            ttl_ms: None,
                        },
                    },
                ],
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn merkle_req_bucket_message_roundtrip() {
        let msg = MerkleReqBucketMessage {
            r#type: "MERKLE_REQ_BUCKET".to_string(),
            payload: MerkleReqBucketPayload {
                map_name: "users".to_string(),
                path: "0/1/2".to_string(),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- ORMap sync messages ----

    #[test]
    fn ormap_sync_init_roundtrip() {
        let mut bucket_hashes = HashMap::new();
        bucket_hashes.insert("0".to_string(), 111.0);
        bucket_hashes.insert("1".to_string(), 222.0);

        let msg = ORMapSyncInit {
            r#type: "ORMAP_SYNC_INIT".to_string(),
            map_name: "tags".to_string(),
            root_hash: 999.0,
            bucket_hashes,
            last_sync_timestamp: Some(1_700_000_000_000.0),
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn ormap_sync_resp_root_roundtrip() {
        let msg = ORMapSyncRespRoot {
            r#type: "ORMAP_SYNC_RESP_ROOT".to_string(),
            payload: ORMapSyncRespRootPayload {
                map_name: "tags".to_string(),
                root_hash: 42.0,
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 0,
                    node_id: "node-1".to_string(),
                },
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn ormap_sync_resp_buckets_roundtrip() {
        let mut buckets = HashMap::new();
        buckets.insert("0".to_string(), 100.0);

        let msg = ORMapSyncRespBuckets {
            r#type: "ORMAP_SYNC_RESP_BUCKETS".to_string(),
            payload: ORMapSyncRespBucketsPayload {
                map_name: "tags".to_string(),
                path: "0".to_string(),
                buckets,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn ormap_merkle_req_bucket_roundtrip() {
        let msg = ORMapMerkleReqBucket {
            r#type: "ORMAP_MERKLE_REQ_BUCKET".to_string(),
            payload: ORMapMerkleReqBucketPayload {
                map_name: "tags".to_string(),
                path: "0/1".to_string(),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn ormap_sync_resp_leaf_roundtrip() {
        let msg = ORMapSyncRespLeaf {
            r#type: "ORMAP_SYNC_RESP_LEAF".to_string(),
            payload: ORMapSyncRespLeafPayload {
                map_name: "tags".to_string(),
                path: "0/1".to_string(),
                entries: vec![
                    ORMapEntry {
                        key: "tag-1".to_string(),
                        records: vec![
                            ORMapRecord {
                                value: rmpv::Value::String("important".into()),
                                timestamp: Timestamp {
                                    millis: 1_700_000_000_000,
                                    counter: 0,
                                    node_id: "node-1".to_string(),
                                },
                                tag: "1700000000000:0:node-1".to_string(),
                                ttl_ms: None,
                            },
                        ],
                        tombstones: vec!["old-tag".to_string()],
                    },
                ],
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn ormap_diff_request_roundtrip() {
        let msg = ORMapDiffRequest {
            r#type: "ORMAP_DIFF_REQUEST".to_string(),
            payload: ORMapDiffRequestPayload {
                map_name: "tags".to_string(),
                keys: vec!["key-1".to_string(), "key-2".to_string()],
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn ormap_diff_response_roundtrip() {
        let msg = ORMapDiffResponse {
            r#type: "ORMAP_DIFF_RESPONSE".to_string(),
            payload: ORMapDiffResponsePayload {
                map_name: "tags".to_string(),
                entries: vec![
                    ORMapEntry {
                        key: "key-1".to_string(),
                        records: vec![],
                        tombstones: vec![],
                    },
                ],
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn ormap_push_diff_roundtrip() {
        let msg = ORMapPushDiff {
            r#type: "ORMAP_PUSH_DIFF".to_string(),
            payload: ORMapPushDiffPayload {
                map_name: "tags".to_string(),
                entries: vec![
                    ORMapEntry {
                        key: "key-1".to_string(),
                        records: vec![
                            ORMapRecord {
                                value: rmpv::Value::Integer(42.into()),
                                timestamp: Timestamp {
                                    millis: 100,
                                    counter: 0,
                                    node_id: "n".to_string(),
                                },
                                tag: "100:0:n".to_string(),
                                ttl_ms: Some(5000),
                            },
                        ],
                        tombstones: vec![],
                    },
                ],
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- Write concern response messages ----

    #[test]
    fn op_ack_message_roundtrip() {
        let msg = OpAckMessage {
            r#type: "OP_ACK".to_string(),
            payload: OpAckPayload {
                last_id: "op-batch-1".to_string(),
                achieved_level: Some(WriteConcern::REPLICATED),
                results: Some(vec![
                    OpResult {
                        op_id: "op-1".to_string(),
                        success: true,
                        achieved_level: WriteConcern::APPLIED,
                        error: None,
                    },
                    OpResult {
                        op_id: "op-2".to_string(),
                        success: false,
                        achieved_level: WriteConcern::MEMORY,
                        error: Some("timeout".to_string()),
                    },
                ]),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn op_ack_message_minimal_roundtrip() {
        let msg = OpAckMessage {
            r#type: "OP_ACK".to_string(),
            payload: OpAckPayload {
                last_id: "op-1".to_string(),
                achieved_level: None,
                results: None,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn op_rejected_message_roundtrip() {
        let msg = OpRejectedMessage {
            r#type: "OP_REJECTED".to_string(),
            payload: OpRejectedPayload {
                op_id: "op-1".to_string(),
                reason: "permission denied".to_string(),
                code: Some(403.0),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn op_rejected_message_without_code_roundtrip() {
        let msg = OpRejectedMessage {
            r#type: "OP_REJECTED".to_string(),
            payload: OpRejectedPayload {
                op_id: "op-2".to_string(),
                reason: "unknown error".to_string(),
                code: None,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- Batched messages ----

    #[test]
    fn batch_message_roundtrip() {
        let msg = BatchMessage {
            r#type: "BATCH".to_string(),
            count: 3.0,
            data: vec![0x00, 0x01, 0x02, 0xFF, 0xFE],
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- AC-4: Optional field omission (byte inspection) ----

    #[test]
    fn sync_init_optional_field_omitted_when_none() {
        // AC-4: When lastSyncTimestamp is None, the key should not appear in serialized output
        let msg = SyncInitMessage {
            r#type: "SYNC_INIT".to_string(),
            map_name: "users".to_string(),
            last_sync_timestamp: None,
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let map = val.as_map().expect("should be a map");

        let has_last_sync = map
            .iter()
            .any(|(k, _)| k.as_str() == Some("lastSyncTimestamp"));
        assert!(
            !has_last_sync,
            "lastSyncTimestamp should be omitted when None"
        );
    }

    #[test]
    fn op_ack_optional_fields_omitted_when_none() {
        // AC-4: When achievedLevel and results are None, those keys should not appear
        let msg = OpAckMessage {
            r#type: "OP_ACK".to_string(),
            payload: OpAckPayload {
                last_id: "op-1".to_string(),
                achieved_level: None,
                results: None,
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let map = val.as_map().expect("should be a map");

        // The payload is nested, so we need to look inside it
        let payload = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");

        let has_achieved = payload_map
            .iter()
            .any(|(k, _)| k.as_str() == Some("achievedLevel"));
        assert!(
            !has_achieved,
            "achievedLevel should be omitted when None"
        );

        let has_results = payload_map
            .iter()
            .any(|(k, _)| k.as_str() == Some("results"));
        assert!(!has_results, "results should be omitted when None");
    }

    // ---- camelCase verification ----

    #[test]
    fn sync_init_camel_case_field_names() {
        let msg = SyncInitMessage {
            r#type: "SYNC_INIT".to_string(),
            map_name: "test".to_string(),
            last_sync_timestamp: Some(100.0),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let map = val.as_map().expect("should be a map");

        let keys: Vec<&str> = map.iter().filter_map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"type"), "expected 'type' field");
        assert!(keys.contains(&"mapName"), "expected camelCase 'mapName'");
        assert!(
            keys.contains(&"lastSyncTimestamp"),
            "expected camelCase 'lastSyncTimestamp'"
        );
    }

    #[test]
    fn batch_message_data_serializes_as_binary() {
        // Verify that data field is serialized as MsgPack bin, not as array of integers
        let msg = BatchMessage {
            r#type: "BATCH".to_string(),
            count: 2.0,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let map = val.as_map().expect("should be a map");

        let data_val = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("data"))
            .map(|(_, v)| v)
            .expect("should have data field");

        // data should be binary, not an array
        assert!(
            data_val.is_bin(),
            "data should serialize as MsgPack bin format, got: {data_val:?}"
        );
        assert_eq!(
            data_val.as_slice(),
            Some(&[0xDE, 0xAD, 0xBE, 0xEF][..]),
            "binary data should match"
        );
    }
}
