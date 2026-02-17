# SPEC-052b: Message Schema -- Sync and Query Domain Structs

---
id: SPEC-052b
type: feature
status: audited
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

### Domain 2: Sync Messages (~25 types)
**Source:** `sync-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `ClientOpMessage` | `ClientOpMessageSchema` | type = "CLIENT_OP", wraps `ClientOp` from base |
| `OpBatchPayload` | `OpBatchPayloadSchema` | ops: `Vec<ClientOp>`, optional writeConcern: WriteConcern, optional timeout: number |
| `OpBatchMessage` | `OpBatchMessageSchema` | type = "OP_BATCH" |
| `SyncInitMessage` | `SyncInitMessageSchema` | type = "SYNC_INIT", mapName + optional lastSyncTimestamp (FLAT - no payload wrapper) |
| `SyncRespRootPayload` | `SyncRespRootPayloadSchema` | mapName, rootHash, timestamp |
| `SyncRespRootMessage` | `SyncRespRootMessageSchema` | type = "SYNC_RESP_ROOT" |
| `SyncRespBucketsPayload` | `SyncRespBucketsPayloadSchema` | mapName, path, buckets (hash map) |
| `SyncRespBucketsMessage` | `SyncRespBucketsMessageSchema` | type = "SYNC_RESP_BUCKETS" |
| `SyncLeafRecord` | inline in `SyncRespLeafMessageSchema` | key, record (LWWRecord) -- not a standalone type, inline in records array |
| `SyncRespLeafPayload` | `SyncRespLeafPayloadSchema` | mapName, path, records (array of { key, record }) |
| `SyncRespLeafMessage` | `SyncRespLeafMessageSchema` | type = "SYNC_RESP_LEAF" |
| `MerkleReqBucketPayload` | `MerkleReqBucketPayloadSchema` | mapName, path |
| `MerkleReqBucketMessage` | `MerkleReqBucketMessageSchema` | type = "MERKLE_REQ_BUCKET" |
| `ORMapEntry` | `ORMapEntrySchema` | Shared sub-type: key + records (array of ORMapRecord) + tombstones (array of string) |
| `ORMapSyncInit` | `ORMapSyncInitSchema` | type = "ORMAP_SYNC_INIT", mapName, rootHash, bucketHashes (record<string, number>), optional lastSyncTimestamp (FLAT - no payload wrapper) |
| `ORMapSyncRespRoot` | `ORMapSyncRespRootSchema` | type = "ORMAP_SYNC_RESP_ROOT", payload: mapName, rootHash, timestamp -- mirrors LWW `SyncRespRootPayload` |
| `ORMapSyncRespBuckets` | `ORMapSyncRespBucketsSchema` | type = "ORMAP_SYNC_RESP_BUCKETS", payload: mapName, path, buckets (record<string, number>) -- mirrors LWW `SyncRespBucketsPayload` |
| `ORMapMerkleReqBucket` | `ORMapMerkleReqBucketSchema` | type = "ORMAP_MERKLE_REQ_BUCKET", payload: mapName, path -- mirrors LWW `MerkleReqBucketPayload` |
| `ORMapSyncRespLeaf` | `ORMapSyncRespLeafSchema` | type = "ORMAP_SYNC_RESP_LEAF", payload: mapName, path, entries (array of ORMapEntry) -- differs from LWW: uses entries/ORMapEntry instead of records/SyncLeafRecord |
| `ORMapDiffRequest` | `ORMapDiffRequestSchema` | type = "ORMAP_DIFF_REQUEST", payload: mapName, keys (array of string) -- unique to ORMap |
| `ORMapDiffResponse` | `ORMapDiffResponseSchema` | type = "ORMAP_DIFF_RESPONSE", payload: mapName, entries (array of ORMapEntry) -- unique to ORMap |
| `ORMapPushDiff` | `ORMapPushDiffSchema` | type = "ORMAP_PUSH_DIFF", payload: mapName, entries (array of ORMapEntry) -- unique to ORMap |
| `OpResult` | `OpResultSchema` | opId, success, achievedLevel (WriteConcern), optional error |
| `OpAckPayload` | `OpAckMessageSchema` payload | lastId, optional achievedLevel (WriteConcern), optional results (array of OpResult) |
| `OpAckMessage` | `OpAckMessageSchema` | type = "OP_ACK" |
| `OpRejectedPayload` | `OpRejectedMessageSchema` payload | opId, reason, optional code (number) |
| `OpRejectedMessage` | `OpRejectedMessageSchema` | type = "OP_REJECTED" |
| `BatchMessage` | `BatchMessageSchema` | type = "BATCH", count, data (binary Vec<u8>) (FLAT - no payload wrapper) |

**FLAT vs PAYLOAD-WRAPPED:**
- **FLAT (no payload wrapper):** `SyncInitMessage`, `ORMapSyncInit`, `BatchMessage` -- fields are directly on the message object
- **PAYLOAD-WRAPPED:** All other messages have a `payload` field containing the nested struct

### Domain 3: Query Messages (~4 types)
**Source:** `query-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `QuerySubPayload` | `QuerySubPayloadSchema` | queryId, mapName, query |
| `QuerySubMessage` | `QuerySubMessageSchema` | type = "QUERY_SUB" |
| `QueryUnsubPayload` | `QueryUnsubPayloadSchema` | queryId |
| `QueryUnsubMessage` | `QueryUnsubMessageSchema` | type = "QUERY_UNSUB" |
| `CursorStatus` | `CursorStatusSchema` | Enum: valid, expired, invalid, none |
| `QueryResultEntry` | inline in `QueryRespPayloadSchema` | key, value (rmpv::Value) -- inline in results array |
| `QueryRespPayload` | `QueryRespPayloadSchema` | queryId, results (array of { key, value }), optional nextCursor, optional hasMore, optional cursorStatus |
| `QueryRespMessage` | `QueryRespMessageSchema` | type = "QUERY_RESP" |

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/sync.rs` | All sync domain structs and enums (~25 types) |
| `packages/core-rust/src/messages/query.rs` | Query sub/unsub/resp structs (~8 types) |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/messages/mod.rs` | Add `pub mod sync;` and `pub mod query;` declarations + re-exports |

**Total: 2 new + 1 modified = 3 files**

## Acceptance Criteria

1. **AC-sync-roundtrip:** All sync domain structs (`ClientOpMessage`, `OpBatchMessage`, `SyncInitMessage`, `SyncRespRootMessage`, `SyncRespBucketsMessage`, `SyncRespLeafMessage`, `MerkleReqBucketMessage`, all 8 ORMap variants, `OpAckMessage`, `OpRejectedMessage`, `BatchMessage`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

2. **AC-query-roundtrip:** All query domain structs (`QuerySubMessage`, `QueryUnsubMessage`, `QueryRespMessage`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

3. **AC-4 (from parent): Optional field omission.** When a Rust struct has `None` for an optional field, the serialized MsgPack does NOT contain that key. Verified by byte inspection in at least 2 representative sync/query message types (e.g., `SyncInitMessage.lastSyncTimestamp`, `QueryRespPayload.nextCursor`).

4. **AC-7 (from parent): cargo test passes.** All existing core-rust tests pass. All new sync/query serde tests pass. No regressions.

## Constraints

- Do NOT implement message handler logic -- strictly struct definitions and serde configuration.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT use `rmp_serde::to_vec()` for wire messages -- always use `rmp_serde::to_vec_named()`.
- Max 5 files modified/created.

## Assumptions

- `ClientOp`, `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>`, `WriteConcern`, `Query` are available from SPEC-052a.
- Most payload structs are nested under a `payload` field (not flattened), matching the TS wire format. **Exceptions:** `SyncInitMessage`, `ORMapSyncInit`, and `BatchMessage` are FLAT (fields directly on message object, no payload wrapper).
- `BatchMessage.data` (Uint8Array) maps to `Vec<u8>` with MsgPack bin format (default for both msgpackr and rmp-serde).
- Inline record types (e.g., `{ key, record }` in `SyncRespLeafMessage`, `{ key, value }` in `QueryRespMessage`) can be defined as nested structs in Rust (e.g., `SyncLeafRecord`, `QueryResultEntry`).

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `messages/sync.rs` with all sync structs (LWW sync + ORMap sync + OpAck/OpRejected/Batch) | -- | ~20% |
| G2 | 1 | Create `messages/query.rs` with all query structs | -- | ~5% |
| G3 | 2 | Update `messages/mod.rs`, add unit tests for round-trip | G1, G2 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

---
*Child of SPEC-052. Created by /sf:split on 2026-02-15.*

## Audit History

### Audit v1 (2026-02-16 14:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~30% total

**Critical:**

1. **OpResult field mismatch with TS source.** The spec says `OpResult` has fields "key, mapName, timestamp, success, optional error" but `OpResultSchema` in `sync-schemas.ts` (line 173-178) actually has `opId, success, achievedLevel, error`. Three fields are wrong (`key`, `mapName`, `timestamp` do not exist) and one field is missing (`achievedLevel: WriteConcern`). An implementer following this spec will produce structs that cannot deserialize TS-produced MsgPack.

2. **QuerySubPayload field mismatch with TS source.** The spec says `QuerySubPayload` has "subscriptionId, mapName, query, optional pageSize/cursor" but `QuerySubMessageSchema` in `query-schemas.ts` (line 6-13) actually has `queryId, mapName, query` -- no `pageSize`, no `cursor`, and the ID field is `queryId` not `subscriptionId`. Same issue with `QueryUnsubPayload` which uses `queryId` not `subscriptionId`.

3. **QueryRespPayload field mismatch with TS source.** The spec says "subscriptionId, results, totalCount, cursor fields" but `QueryRespPayloadSchema` (line 28-37) actually has `queryId, results, nextCursor, hasMore, cursorStatus`. The field `totalCount` does not exist, the ID field is `queryId` not `subscriptionId`, and specific cursor fields (`nextCursor`, `hasMore`, `cursorStatus`) are not named.

4. **SyncRespBucketsPayload missing `path` field.** The spec says "mapName, buckets (hash map)" but `SyncRespBucketsMessageSchema` (line 47-54) also has `path: z.string()`. Without this field, the struct will not round-trip correctly.

5. **SyncLeafRecord phantom type.** The spec lists `SyncLeafRecord` with "key, value (LWWRecord), optional tombstone" but the TS source `SyncRespLeafMessageSchema` (line 57-67) uses an inline object `{ key: string, record: LWWRecord }` with no `tombstone` field. The field is called `record` not `value`, and there is no `tombstone`.

6. **SyncRespLeafPayload and MerkleReqBucketPayload use "prefix" but TS uses "path".** The TS source uses `path` as the field name in `SyncRespLeafMessageSchema` (line 59) and `MerkleReqBucketMessageSchema` (line 73). Since serde must match the TS wire format exactly, using "prefix" in the Notes column will mislead implementers.

7. **ORMapSyncInit is a flat struct, not payload-wrapped.** The TS `ORMapSyncInitSchema` (line 97-103) has fields directly on the message object (`type, mapName, rootHash, bucketHashes, lastSyncTimestamp`) without a `payload` wrapper, unlike most other ORMap messages. The spec assumption "Payload structs are nested under a payload field (not flattened)" is violated by this type and by `SyncInitMessage`. The implementer needs to know which messages are flat vs. payload-wrapped.

**Recommendations:**

8. **ORMapEntry naming.** The spec calls it `ORMapLeafEntry` referencing `ORMapLeafEntrySchema`, but the TS source names it `ORMapEntrySchema` / `ORMapEntry` (line 84-89). Consider using the TS name `ORMapEntry` to maintain consistency.

9. **OpBatchPayload has additional fields.** The spec says `OpBatchPayload` has "ops: Vec<ClientOp>" but the TS `OpBatchMessageSchema` payload (line 18-25) also has optional `writeConcern` and `timeout` fields. These should be documented.

10. **G1 and G2 both write to `sync.rs`.** Having two parallel workers (G1 and G2) both creating/writing to the same file `sync.rs` will cause conflicts. Either combine them into one group or split into two separate files.

11. **BatchMessage has `count` field.** The spec notes only "binary data (Vec<u8>)" but the TS `BatchMessageSchema` (line 208-213) also has `count: z.number()`. This field must be included.

### Response v1 (2026-02-16 15:30)
**Applied:** All 11 items (7 critical + 4 recommendations)

**Changes:**

1. [✓] **OpResult field mismatch** — Fixed Requirements table: changed "key, mapName, timestamp, success, optional error" to "opId, success, achievedLevel (WriteConcern), optional error" to match `OpResultSchema` (line 173-178 of sync-schemas.ts).

2. [✓] **QuerySubPayload field mismatch** — Fixed Requirements table: changed "subscriptionId, mapName, query, optional pageSize/cursor" to "queryId, mapName, query" to match `QuerySubPayloadSchema` (line 8-12 of query-schemas.ts).

3. [✓] **QueryRespPayload field mismatch** — Fixed Requirements table: changed "subscriptionId, results, totalCount, cursor fields" to "queryId, results (array of { key, value }), optional nextCursor, optional hasMore, optional cursorStatus" to match `QueryRespPayloadSchema` (line 28-37 of query-schemas.ts).

4. [✓] **SyncRespBucketsPayload missing path** — Added `path` field to Requirements table Notes column: "mapName, path, buckets (hash map)" to match `SyncRespBucketsMessageSchema` (line 47-54 of sync-schemas.ts).

5. [✓] **SyncLeafRecord phantom type** — Fixed Requirements table: changed "key, value (LWWRecord), optional tombstone" to "key, record (LWWRecord) -- not a standalone type, inline in records array" and clarified that it's inline in the TS schema (line 61-64 of sync-schemas.ts), not a top-level export. Also fixed `SyncRespLeafPayload` Notes to "mapName, path, records (array of { key, record })".

6. [✓] **prefix→path naming** — Changed all instances of "prefix" to "path" in Requirements table Notes for `SyncRespLeafPayload` and `MerkleReqBucketPayload` to match TS source field names.

7. [✓] **Flat vs payload-wrapped ambiguity** — Added explicit documentation: (a) Added "(FLAT - no payload wrapper)" note to `SyncInitMessage` and `ORMapSyncInit` in Requirements table. (b) Added subsection "FLAT vs PAYLOAD-WRAPPED" under Domain 2 table documenting which messages are flat. (c) Updated Assumptions section to explicitly note the two exceptions (`SyncInitMessage` and `ORMapSyncInit`).

8. [✓] **ORMapEntry naming** — Renamed `ORMapLeafEntry` → `ORMapEntry` in Requirements table to match TS source name `ORMapEntrySchema` (line 84-89 of sync-schemas.ts).

9. [✓] **OpBatchPayload optional fields** — Added "optional writeConcern: WriteConcern, optional timeout: number" to `OpBatchPayload` Notes column to match `OpBatchMessageSchema` payload (line 18-25 of sync-schemas.ts).

10. [✓] **G1+G2 file conflict** — Merged G1 and G2 into single group G1 (Wave 1, ~20% context) to avoid parallel writes to `sync.rs`. Updated Task Groups table and Execution Plan: now 2 workers max (G1 + G2 in Wave 1), G3 in Wave 2.

11. [✓] **BatchMessage count field** — Added "count" field to `BatchMessage` Notes in Requirements table: "type = "BATCH", count, data (binary Vec<u8>)" to match `BatchMessageSchema` (line 208-213 of sync-schemas.ts).

**Additional changes:**
- Updated `QueryResultEntry` Notes to clarify it's "inline in results array" (similar to `SyncLeafRecord`).
- Updated Acceptance Criteria AC-3 example field from `QuerySubPayload.cursor` → `QueryRespPayload.nextCursor` (since QuerySubPayload has no cursor field).
- Updated Assumptions section to document inline record types pattern.

### Files Modified

- .specflow/specs/SPEC-052b.md

### Next Step

`/sf:audit` — re-audit revised specification

Tip: `/clear` recommended — auditor needs fresh context

### Audit v2 (2026-02-16 17:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~30% total

**Critical:**

1. **OpAckMessage payload fields undocumented.** The spec Notes column says only `type = "OP_ACK"` but the TS source `OpAckMessageSchema` (sync-schemas.ts line 181-188) has a payload with three fields: `lastId: z.string()`, `achievedLevel: WriteConcernSchema.optional()`, and `results: z.array(OpResultSchema).optional()`. Without documenting these fields, an implementer cannot create the correct `OpAckPayload` struct. This is the same class of error as the original Audit v1 field-mismatch issues.

2. **OpRejectedMessage payload fields undocumented.** The spec Notes column says only `type = "OP_REJECTED"` but the TS source `OpRejectedMessageSchema` (sync-schemas.ts line 191-198) has a payload with three fields: `opId: z.string()`, `reason: z.string()`, and `code: z.number().optional()`. Same issue as above.

3. **BatchMessage is FLAT but not listed in FLAT vs PAYLOAD-WRAPPED section.** The TS source `BatchMessageSchema` (sync-schemas.ts line 208-213) has `type`, `count`, and `data` directly on the message object with no `payload` wrapper. The spec's "FLAT vs PAYLOAD-WRAPPED" section lists only `SyncInitMessage` and `ORMapSyncInit` as FLAT. The Assumptions section also only lists two FLAT exceptions. An implementer following the documented rule ("All other messages have a payload field") would incorrectly wrap BatchMessage fields in a payload struct.

**Recommendations:**

4. **Add OpAckPayload and OpRejectedPayload rows.** For consistency with `OpBatchPayload` (which is listed as a separate row from `OpBatchMessage`), consider adding explicit `OpAckPayload` and `OpRejectedPayload` rows to the Requirements table with their field details, rather than cramming all fields into the Notes column of the Message row.

5. **Type count label "~16 types" is inaccurate.** The actual number of distinct Rust types in the sync domain (counting message types, payload types, sub-types, and all 8 ORMap variants individually) is approximately 25. While the table itself enumerates all types correctly, the heading is misleading. Consider updating to "~25 types" or removing the approximate count.

### Response v2 (2026-02-16 18:30)
**Applied:** All 5 items (3 critical + 2 recommendations)

**Changes:**

1. [✓] **OpAckMessage payload fields undocumented** — Added explicit `OpAckPayload` row to Requirements table with fields "lastId, optional achievedLevel (WriteConcern), optional results (array of OpResult)" to match `OpAckMessageSchema` payload (sync-schemas.ts line 183-187). Updated `OpAckMessage` Notes to reference payload.

2. [✓] **OpRejectedMessage payload fields undocumented** — Added explicit `OpRejectedPayload` row to Requirements table with fields "opId, reason, optional code (number)" to match `OpRejectedMessageSchema` payload (sync-schemas.ts line 193-197). Updated `OpRejectedMessage` Notes to reference payload.

3. [✓] **BatchMessage FLAT not listed** — Updated "FLAT vs PAYLOAD-WRAPPED" section to include `BatchMessage` in FLAT list: "**FLAT (no payload wrapper):** `SyncInitMessage`, `ORMapSyncInit`, `BatchMessage`". Also updated Assumptions section to list all three FLAT exceptions.

4. [✓] **Add OpAckPayload and OpRejectedPayload rows** — Applied as part of items 1 and 2 above. Added explicit payload struct rows for consistency with `OpBatchPayload` pattern.

5. [✓] **Type count label inaccurate** — Updated heading from "~16 types" to "~25 types" to accurately reflect the total number of distinct Rust types in the sync domain (including message types, payload types, sub-types, and all 8 ORMap variants).

### Files Modified

- .specflow/specs/SPEC-052b.md

### Next Step

`/sf:audit` — re-audit revised specification

Tip: `/clear` recommended — auditor needs fresh context

### Audit v3 (2026-02-16 19:30)
**Status:** APPROVED

**Context Estimate:** ~30% total

**Dimensions Evaluated:**
- Clarity: All types, fields, and flat/payload-wrapped distinctions are clearly documented.
- Completeness: All LWW sync, ORMap sync, write concern, batch, and query types are enumerated with field details. Files to create/modify are listed.
- Testability: All 4 acceptance criteria are measurable (round-trip tests, byte inspection, cargo test).
- Scope: 3 files (2 new + 1 modified), well within 5-file Rust language profile limit. Constraints clearly state no handler logic.
- Feasibility: Straightforward serde struct definitions with established patterns from SPEC-052a.
- Architecture fit: Follows existing `messages/base.rs` patterns (camelCase, named encoding, skip_serializing_if).
- Non-duplication: No overlap with SPEC-052a base types or future SPEC-052c/d domains.
- Cognitive load: Simple struct definitions, no complex logic. Clear reference to TS source files.
- Strategic fit: Aligned with Rust migration Phase 2 goals and MsgPack wire protocol decision.
- Project compliance: Honors all PROJECT.md decisions (MsgPack, Rust language profile, max 5 files).

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~30% | <=50% | OK |
| Largest task group (G1) | ~20% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** GOOD range (30-50%)

**Language Profile:** Compliant with Rust profile (3 files <= 5 max; all groups define types only, satisfying trait-first)

**Strategic fit:** Aligned with project goals

**Project compliance:** Honors PROJECT.md decisions

**Recommendations:**

1. **ORMap variant fields not individually documented.** The 8 ORMap variants are listed in a single aggregate row without per-variant field details. While 5 mirror LWW counterparts documented above, 3 unique types (ORMapDiffRequest: mapName + keys; ORMapDiffResponse: mapName + entries; ORMapPushDiff: mapName + entries) and the FLAT ORMapSyncInit (mapName, rootHash, bucketHashes, lastSyncTimestamp) require the implementer to consult the TS source directly. Consider expanding the aggregate row into individual rows with field notes, or adding a brief field summary for the non-LWW-mirror variants.

**Comment:** Spec is well-structured after two revision cycles. All field names verified against `sync-schemas.ts` and `query-schemas.ts`. FLAT vs payload-wrapped distinction is clearly documented. Acceptance criteria are concrete and testable. Ready for implementation.

### Response v3 (2026-02-16 20:30)
**Applied:** Recommendation 1 from Audit v3

**Changes:**

1. [✓] **ORMap variant fields not individually documented** -- Replaced the single aggregate row ("8 ORMap sync variants") with 8 individual rows in the Domain 2 Requirements table. Each row includes the Rust type name, TS schema source, and field details verified against `sync-schemas.ts` (lines 97-170). Specifically:
   - `ORMapSyncInit` (line 97-103): FLAT message with mapName, rootHash, bucketHashes (record<string, number>), optional lastSyncTimestamp
   - `ORMapSyncRespRoot` (line 106-113): payload mirrors LWW `SyncRespRootPayload` (mapName, rootHash, timestamp)
   - `ORMapSyncRespBuckets` (line 116-123): payload mirrors LWW `SyncRespBucketsPayload` (mapName, path, buckets)
   - `ORMapMerkleReqBucket` (line 126-132): payload mirrors LWW `MerkleReqBucketPayload` (mapName, path)
   - `ORMapSyncRespLeaf` (line 135-142): payload: mapName, path, entries (array of ORMapEntry) -- differs from LWW (uses entries/ORMapEntry instead of records/SyncLeafRecord)
   - `ORMapDiffRequest` (line 145-151): payload: mapName, keys (array of string) -- unique to ORMap
   - `ORMapDiffResponse` (line 154-160): payload: mapName, entries (array of ORMapEntry) -- unique to ORMap
   - `ORMapPushDiff` (line 163-169): payload: mapName, entries (array of ORMapEntry) -- unique to ORMap
   - Moved `ORMapEntry` row before the ORMap variants for logical reading order (shared sub-type used by ORMapSyncRespLeaf, ORMapDiffResponse, and ORMapPushDiff)

### Audit v4 (2026-02-16 21:30)
**Status:** APPROVED

**Context Estimate:** ~30% total

**Dimensions Evaluated:**
- Clarity: Title, context, goal, and task are unambiguous. FLAT vs PAYLOAD-WRAPPED distinction is explicitly documented with all three FLAT messages identified. Every type row includes field-level detail.
- Completeness: All 28 sync types and 8 query types verified field-by-field against `sync-schemas.ts` (214 lines) and `query-schemas.ts` (44 lines). Files to create/modify listed. No missing types or fields.
- Testability: All 4 acceptance criteria are measurable: round-trip tests (AC-1, AC-2), byte inspection for optional field omission (AC-3), cargo test pass (AC-4).
- Scope: 3 files (2 new + 1 modified), well within 5-file Rust language profile limit. Constraints clearly bound the work to struct definitions only.
- Feasibility: Straightforward serde struct definitions following established patterns from SPEC-052a (`base.rs`).
- Architecture fit: Follows existing `messages/base.rs` patterns exactly (camelCase, `to_vec_named()`, `skip_serializing_if`, `rmpv::Value` for dynamic fields).
- Non-duplication: No overlap with SPEC-052a base types or future SPEC-052c/d/e domains.
- Cognitive load: Simple struct definitions with no complex logic. Clear TS source references.
- Strategic fit: Aligned with Phase 2 Rust migration goals and MsgPack wire protocol decision.
- Project compliance: Honors all PROJECT.md decisions (MsgPack serialization, Rust language profile, max 5 files per spec, trait-first ordering).

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~30% | <=50% | OK |
| Largest task group (G1) | ~20% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** GOOD range (30-50%)

**Language Profile:** Compliant with Rust profile (3 files <= 5 max; G1 and G2 define types only in Wave 1, G3 wires modules and adds tests in Wave 2)

**Strategic fit:** Aligned with project goals

**Project compliance:** Honors PROJECT.md decisions

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | SPEC-052a base types are available and correct | Compilation failure; blocked until 052a is fixed |
| A2 | TS wire format is stable (no concurrent changes) | Field mismatches; would need spec revision |
| A3 | `rmpv::Value` handles `z.unknown()` correctly | Query result values may fail deserialization |
| A4 | `Vec<u8>` maps to MsgPack bin format for `BatchMessage.data` | Binary data corruption on wire |

All assumptions are reasonable. A1 is verified (SPEC-052a is completed). A2/A3/A4 are validated by the existing SPEC-052a patterns and tests.

**Recommendations:**

1. **Query domain heading says "~4 types" but table has 8 types.** The heading "Domain 3: Query Messages (~4 types)" undercounts the actual Rust types needed (QuerySubPayload, QuerySubMessage, QueryUnsubPayload, QueryUnsubMessage, CursorStatus, QueryResultEntry, QueryRespPayload, QueryRespMessage = 8). The "Files to Create" section correctly says "~8 types". Consider updating the heading to "~8 types" for consistency.

**Comment:** Specification is thorough and implementation-ready after three revision cycles. All fields in both domains verified against the TypeScript source files with zero discrepancies. The FLAT vs PAYLOAD-WRAPPED documentation, explicit payload struct rows, and per-variant ORMap field details make this spec self-contained -- an implementer should not need to consult the TS source for field names or structure.

## Execution Summary

**Executed:** 2026-02-16
**Mode:** orchestrated (sequential fallback)
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Created
- `packages/core-rust/src/messages/sync.rs` -- 28 sync domain structs (LWW sync, ORMap sync, OpAck/OpRejected, BatchMessage)
- `packages/core-rust/src/messages/query.rs` -- 8 query domain structs (QuerySub, QueryUnsub, QueryResp, CursorStatus)

### Files Modified
- `packages/core-rust/src/messages/mod.rs` -- added `pub mod sync;` and `pub mod query;` declarations + re-exports
- `packages/core-rust/Cargo.toml` -- added `serde_bytes = "0.11"` dependency for BatchMessage binary data
- `Cargo.lock` -- updated with serde_bytes

### Acceptance Criteria Status
- [x] AC-sync-roundtrip: All sync domain structs round-trip through `to_vec_named()` / `from_slice()` (26 tests)
- [x] AC-query-roundtrip: All query domain structs round-trip through `to_vec_named()` / `from_slice()` (7 tests)
- [x] AC-4: Optional field omission verified by byte inspection for `SyncInitMessage.lastSyncTimestamp`, `OpAckPayload.achievedLevel/results`, `QueryRespPayload.nextCursor/hasMore/cursorStatus`
- [x] AC-7: cargo test passes -- 232 tests (199 existing + 33 new), zero regressions

### Deviations
1. Added `serde_bytes` dependency (0.11) to ensure `BatchMessage.data` (Vec<u8>) serializes as MsgPack bin format instead of array of integers. This matches TS `Uint8Array` wire behavior.
2. Created separate ORMap payload structs (e.g., `ORMapSyncRespRootPayload`) rather than reusing LWW payload structs, for clarity and future-proofing even though shapes are identical.
3. Module declarations in `mod.rs` were added in Wave 1 (needed for compilation verification) rather than Wave 2 as planned. Re-exports and tests were added in Wave 2 as planned.
