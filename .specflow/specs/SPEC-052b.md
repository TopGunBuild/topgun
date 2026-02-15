# SPEC-052b: Message Schema -- Sync and Query Domain Structs

---
id: SPEC-052b
type: feature
status: blocked
priority: P0
complexity: small
created: 2026-02-15
parent: SPEC-052
depends_on: [SPEC-052a]
todo_ref: TODO-062
---

## Context

This sub-spec implements Rust serde structs for the Sync and Query message domains. These are the most message-dense domains in the TopGun wire protocol, covering all LWW/ORMap synchronization messages and query subscription lifecycle.

All types depend on base types from SPEC-052a (`ClientOp`, `Timestamp`, `LWWRecord`, `ORMapRecord`, `WriteConcern`, `Query`).

### Critical Compatibility Issues (Inherited)

1. **Named encoding:** Must use `rmp_serde::to_vec_named()` for wire messages.
2. **camelCase:** Every struct needs `#[serde(rename_all = "camelCase")]`.
3. **Optional fields:** `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none", default)]`.
4. **Dynamic values:** `rmpv::Value` for `z.any()` / `z.unknown()` fields.
5. **`BatchMessage.data`:** `Uint8Array` in TS maps to `Vec<u8>` in Rust with MsgPack bin format.

## Goal

Implement all Sync and Query domain message structs so they can be deserialized from TS-produced MsgPack and re-serialized to TS-decodable MsgPack.

## Task

Create `messages/sync.rs` and `messages/query.rs` with all structs from `sync-schemas.ts` and `query-schemas.ts`. Register both submodules in `messages/mod.rs`.

### Approach

1. Create `messages/sync.rs` with all sync domain structs.
2. Create `messages/query.rs` with all query domain structs.
3. Update `messages/mod.rs` to declare and re-export both submodules.
4. Add unit tests for serde round-trip of representative structs.

## Requirements

### Domain 2: Sync Messages (~16 types)
**Source:** `sync-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `ClientOpMessage` | `ClientOpMessageSchema` | type = "CLIENT_OP", wraps `ClientOp` from base |
| `OpBatchPayload` | `OpBatchPayloadSchema` | ops: `Vec<ClientOp>` |
| `OpBatchMessage` | `OpBatchMessageSchema` | type = "OP_BATCH" |
| `SyncInitMessage` | `SyncInitMessageSchema` | type = "SYNC_INIT", mapName + optional lastSyncTimestamp |
| `SyncRespRootPayload` | `SyncRespRootPayloadSchema` | mapName, rootHash, timestamp |
| `SyncRespRootMessage` | `SyncRespRootMessageSchema` | type = "SYNC_RESP_ROOT" |
| `SyncRespBucketsPayload` | `SyncRespBucketsPayloadSchema` | mapName, buckets (hash map) |
| `SyncRespBucketsMessage` | `SyncRespBucketsMessageSchema` | type = "SYNC_RESP_BUCKETS" |
| `SyncLeafRecord` | `SyncLeafRecordSchema` | key, value (LWWRecord), optional tombstone |
| `SyncRespLeafPayload` | `SyncRespLeafPayloadSchema` | mapName, prefix, records |
| `SyncRespLeafMessage` | `SyncRespLeafMessageSchema` | type = "SYNC_RESP_LEAF" |
| `MerkleReqBucketPayload` | `MerkleReqBucketPayloadSchema` | mapName, prefix |
| `MerkleReqBucketMessage` | `MerkleReqBucketMessageSchema` | type = "MERKLE_REQ_BUCKET" |
| 8 ORMap sync variants | ORMap*Schema | ORMAP_SYNC_INIT, ORMAP_SYNC_RESP_ROOT, ORMAP_SYNC_RESP_BUCKETS, ORMAP_MERKLE_REQ_BUCKET, ORMAP_SYNC_RESP_LEAF, ORMAP_DIFF_REQUEST, ORMAP_DIFF_RESPONSE, ORMAP_PUSH_DIFF |
| `ORMapLeafEntry` | `ORMapLeafEntrySchema` | Shared sub-type: key + records + tombstones |
| `OpResult` | `OpResultSchema` | key, mapName, timestamp, success, optional error |
| `OpAckMessage` | `OpAckMessageSchema` | type = "OP_ACK" |
| `OpRejectedMessage` | `OpRejectedMessageSchema` | type = "OP_REJECTED" |
| `BatchMessage` | `BatchMessageSchema` | type = "BATCH", binary data (`Vec<u8>`) |

### Domain 3: Query Messages (~4 types)
**Source:** `query-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `QuerySubPayload` | `QuerySubPayloadSchema` | subscriptionId, mapName, query, optional pageSize/cursor |
| `QuerySubMessage` | `QuerySubMessageSchema` | type = "QUERY_SUB" |
| `QueryUnsubPayload` | `QueryUnsubPayloadSchema` | subscriptionId |
| `QueryUnsubMessage` | `QueryUnsubMessageSchema` | type = "QUERY_UNSUB" |
| `CursorStatus` | `CursorStatusSchema` | Enum: valid, expired, invalid, none |
| `QueryResultEntry` | `QueryResultEntrySchema` | key, value (rmpv::Value) |
| `QueryRespPayload` | `QueryRespPayloadSchema` | subscriptionId, results, totalCount, cursor fields |
| `QueryRespMessage` | `QueryRespMessageSchema` | type = "QUERY_RESP" |

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/sync.rs` | All sync domain structs and enums (~16 types) |
| `packages/core-rust/src/messages/query.rs` | Query sub/unsub/resp structs (~8 types) |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/messages/mod.rs` | Add `pub mod sync;` and `pub mod query;` declarations + re-exports |

**Total: 2 new + 1 modified = 3 files**

## Acceptance Criteria

1. **AC-sync-roundtrip:** All sync domain structs (`ClientOpMessage`, `OpBatchMessage`, `SyncInitMessage`, `SyncRespRootMessage`, `SyncRespBucketsMessage`, `SyncRespLeafMessage`, `MerkleReqBucketMessage`, all 8 ORMap variants, `OpAckMessage`, `OpRejectedMessage`, `BatchMessage`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

2. **AC-query-roundtrip:** All query domain structs (`QuerySubMessage`, `QueryUnsubMessage`, `QueryRespMessage`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

3. **AC-4 (from parent): Optional field omission.** When a Rust struct has `None` for an optional field, the serialized MsgPack does NOT contain that key. Verified by byte inspection in at least 2 representative sync/query message types (e.g., `SyncInitMessage.lastSyncTimestamp`, `QuerySubPayload.cursor`).

4. **AC-7 (from parent): cargo test passes.** All existing core-rust tests pass. All new sync/query serde tests pass. No regressions.

## Constraints

- Do NOT implement message handler logic -- strictly struct definitions and serde configuration.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT use `rmp_serde::to_vec()` for wire messages -- always use `rmp_serde::to_vec_named()`.
- Max 5 files modified/created.

## Assumptions

- `ClientOp`, `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>`, `WriteConcern`, `Query` are available from SPEC-052a.
- Payload structs are nested under a `payload` field (not flattened), matching the TS wire format.
- `BatchMessage.data` (Uint8Array) maps to `Vec<u8>` with MsgPack bin format (default for both msgpackr and rmp-serde).

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `messages/sync.rs` with all LWW sync structs (SYNC_INIT through MERKLE_REQ_BUCKET) | -- | ~10% |
| G2 | 1 | Create ORMap sync structs in `sync.rs` (8 variants + ORMapLeafEntry) and OpAck/OpRejected/Batch | -- | ~10% |
| G3 | 1 | Create `messages/query.rs` with all query structs | -- | ~5% |
| G4 | 2 | Update `messages/mod.rs`, add unit tests for round-trip | G1, G2, G3 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3 | Yes | 3 |
| 2 | G4 | No | 1 |

**Total workers needed:** 3 (max in any wave)

---
*Child of SPEC-052. Created by /sf:split on 2026-02-15.*
