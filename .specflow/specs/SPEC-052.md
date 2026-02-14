# SPEC-052: Message Schema Compatibility -- Rust Serde Structs for MsgPack Wire Protocol

---
id: SPEC-052
type: feature
status: draft
priority: P0
complexity: large
created: 2026-02-14
depends_on: [SPEC-049]
todo_ref: TODO-062
---

## Context

The Rust server must serialize and deserialize every message type that the TypeScript client sends and receives over the MsgPack wire protocol. The TS Zod schemas in `packages/core/src/schemas/` are the source of truth for the wire format. Rust `serde` structs must produce byte-identical MsgPack output when round-tripping through `rmp_serde::to_vec_named()` / `rmp_serde::from_slice()`.

This is a P0 (client-server contract) task because any incompatibility between Rust and TS serialization silently corrupts data or drops messages at runtime. There are no type-checker guardrails across the language boundary -- only cross-language integration tests can verify correctness.

### Critical Compatibility Issues

1. **Named vs positional encoding:** `msgpackr.pack()` serializes JS objects as MsgPack maps with string keys. The default `rmp_serde::to_vec()` serializes Rust structs as MsgPack ARRAYS (positional). **Must use `rmp_serde::to_vec_named()`** (or the `Serializer::with_struct_map()` config) to produce named-field maps.

2. **Field naming:** TS uses camelCase (`nodeId`, `mapName`, `writeConcern`). Rust convention is snake_case. Every struct needs `#[serde(rename_all = "camelCase")]` or per-field `#[serde(rename = "...")]`.

3. **Discriminated union:** TS uses `{ type: "AUTH", ... }` with `z.discriminatedUnion('type', [...])`. Rust needs `#[serde(tag = "type")]` for internally-tagged enum representation matching this wire format.

4. **Optional fields:** TS `.optional()` fields are omitted from the wire when absent. Rust must use `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]` to avoid sending `null` for absent fields. Additionally, `#[serde(default)]` is needed on `Option<T>` fields so deserialization tolerates missing keys.

5. **Dynamic values:** TS `z.any()` / `z.unknown()` fields need a Rust representation. The existing `Value` enum handles structured data but does not directly roundtrip with arbitrary JS values. Use `rmpv::Value` for true dynamic MsgPack values at the wire boundary.

6. **Existing Rust types need migration:** `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` in `hlc.rs` currently serialize with snake_case field names. These must gain `#[serde(rename_all = "camelCase")]` to match the TS wire format.

### Recent Protocol Cleanup (2026-02-14)

Before this spec was drafted, a protocol audit identified and fixed several issues (commit `53b60e4`):
- **eventType `UPDATED` → `PUT`**: Server was sending a value not in the schema enum; now uses `PUT`/`REMOVE`/`OR_ADD`/`OR_REMOVE` only.
- **HYBRID_QUERY_RESP/DELTA removed**: Dead code (server never sent these). Schemas and handlers deleted.
- **AuthAckMessageSchema added**: `{ type: "AUTH_ACK", protocolVersion?: number }`.
- **ErrorMessageSchema added**: `{ type: "ERROR", payload: { code, message, details? } }`.
- **LOCK schemas fixed**: `LockGrantedPayload` and `LockReleasedPayload` now include `name: string` (server already sent this field).
- **protocolVersion added**: To both `AuthMessageSchema` and `AuthAckMessageSchema` for future protocol negotiation.
- **OP_REJECTED/ERROR handlers added**: Client SyncEngine now handles these server messages.

These fixes mean the TS schemas are now the correct and complete source of truth for all wire types.

### Scale

There are 8 TS schema files defining:
- **44 message variants** in the `MessageSchema` discriminated union (WebSocket transport)
- **~15 additional message types** NOT in the union (server-to-client events, cluster search, HTTP sync, counters)
- **~20 shared sub-types** (payloads, records, enums) used across messages

Total: approximately **57+ distinct Rust struct/enum definitions** across all schema domains (reduced from ~60 after HYBRID_QUERY removal).

### Existing Rust Types

The following types already exist in `packages/core-rust/src/` and need serde rename attributes added:
- `Timestamp` (hlc.rs) -- needs `#[serde(rename_all = "camelCase")]`
- `LWWRecord<V>` (hlc.rs) -- needs `#[serde(rename_all = "camelCase")]`
- `ORMapRecord<V>` (hlc.rs) -- needs `#[serde(rename_all = "camelCase")]`
- `Value` (types.rs) -- already compatible (enum variant names, not field names)
- `MapType` (types.rs) -- may need rename to match TS `"lww"` / `"or"` strings

## Goal

**Outcome:** The Rust server can decode any MsgPack message produced by the TS client, and the TS client can decode any MsgPack message produced by the Rust server, for all 59+ message types in the protocol.

### Observable Truths

1. A TS client can `pack()` any `MessageSchema` variant, and the Rust server can `rmp_serde::from_slice()` it into the correct Rust enum variant.
2. The Rust server can `rmp_serde::to_vec_named()` any message struct, and the TS client can `unpack()` it and pass Zod validation.
3. Round-trip fidelity: TS encode -> Rust decode -> Rust re-encode -> TS decode produces identical data for all message types.
4. Optional fields absent in TS produce no key in MsgPack; Rust deserializes these as `None`. Conversely, Rust `None` fields produce no key; TS sees them as `undefined`.
5. The existing `Timestamp`, `LWWRecord`, `ORMapRecord` types maintain backward compatibility for Rust-only code (CRDT operations) while gaining wire-compatible serialization.

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `packages/core-rust/src/messages/mod.rs` | Module root, re-exports, `Message` discriminated union enum |
| `packages/core-rust/src/messages/base.rs` | Base types: `WriteConcern`, `Timestamp` (re-export with serde), `ClientOp`, `Predicate`, `Query` |
| `packages/core-rust/src/messages/sync.rs` | Sync domain: CLIENT_OP through BATCH, all LWW and ORMap sync messages |
| `packages/core-rust/src/messages/query.rs` | Query domain: QUERY_SUB, QUERY_UNSUB, QUERY_RESP |
| `packages/core-rust/src/messages/search.rs` | Search domain: SEARCH, SEARCH_RESP, SEARCH_SUB, SEARCH_UPDATE, SEARCH_UNSUB |
| `packages/core-rust/src/messages/cluster.rs` | Cluster domain: PARTITION_MAP_REQUEST, CLUSTER_SUB_*, CLUSTER_SEARCH_* |
| `packages/core-rust/src/messages/messaging.rs` | Messaging domain: TOPIC_*, LOCK_*, COUNTER_*, PING/PONG, EntryProcess*, Journal*, ConflictResolver*, MERGE_REJECTED |
| `packages/core-rust/src/messages/client_events.rs` | Server-to-client: SERVER_EVENT, SERVER_BATCH_EVENT, QUERY_UPDATE, GC_PRUNE, AUTH_FAIL, HybridQuery*, Lock*, SyncReset |
| `packages/core-rust/src/messages/http_sync.rs` | HTTP sync: HttpSyncRequest, HttpSyncResponse and their sub-types |
| `packages/core-rust/src/hlc.rs` | Modified: add `#[serde(rename_all = "camelCase")]` to Timestamp, LWWRecord, ORMapRecord |
| `packages/core-rust/Cargo.toml` | Modified: add `rmpv` dependency for dynamic values |
| `packages/core-rust/tests/cross_lang_compat.rs` | Integration test: golden-file byte comparison and round-trip tests |
| `packages/core/src/__tests__/cross-lang-fixtures.test.ts` | TS fixture generator: produces golden MsgPack files for Rust to verify |

### Key Links (Fragile Connections)

1. **Timestamp serde rename** -- Changing `Timestamp` field names affects ALL existing Rust CRDT tests that use `rmp_serde::to_vec()` (not `to_vec_named()`). The array-format serialization is unaffected by `rename_all`, but named-format output changes. Existing tests using `rmp_serde::to_vec()` continue to work; only `to_vec_named()` produces different bytes.

2. **Discriminated union `type` field** -- The Rust `Message` enum uses `#[serde(tag = "type")]`. If any sub-struct also has a field named `type` (e.g., `SearchUpdatePayload.type`, `ClusterSubRegisterPayload.type`), there is a conflict. These inner `type` fields must be renamed (e.g., `#[serde(rename = "type")]` on a Rust field named `update_type` or `sub_type`).

3. **`rmpv::Value` vs `Value` enum** -- The existing `Value` enum in `types.rs` is for CRDT map entries. The `rmpv::Value` type is for arbitrary MsgPack values at the wire boundary (`z.any()`, `z.unknown()` fields). These are different types serving different purposes and must not be confused.

## Task

Create Rust serde structs for ALL TopGun message types, organized by schema domain, with cross-language MsgPack compatibility verified by integration tests.

### Approach

1. **Organize by domain:** One Rust file per TS schema file (base, sync, query, search, cluster, messaging, client-events, http-sync), plus a `mod.rs` with the top-level `Message` enum.

2. **Serde configuration:** Every struct uses `#[serde(rename_all = "camelCase")]`. The top-level `Message` enum uses `#[serde(tag = "type")]` for internally-tagged representation.

3. **Dynamic values:** Use `rmpv::Value` for `z.any()` / `z.unknown()` fields. This preserves arbitrary MsgPack values without loss.

4. **Optional fields:** All `Option<T>` fields use both `#[serde(skip_serializing_if = "Option::is_none")]` and `#[serde(default)]`.

5. **Existing type migration:** Add `#[serde(rename_all = "camelCase")]` to `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` in `hlc.rs`.

6. **Verification strategy:** Golden-file cross-language tests:
   - TS test generates fixture files: for each message type, `msgpackr.pack()` a representative instance and write to `fixtures/<TYPE>.msgpack` + `fixtures/<TYPE>.json` (human-readable reference).
   - Rust integration test reads fixture files, deserializes with `rmp_serde::from_slice()`, re-serializes with `rmp_serde::to_vec_named()`, compares bytes or round-trips through TS again.
   - Additionally: Rust unit tests verify serde round-trip for every struct independently.

## Requirements

### Schema Domain Breakdown

#### Domain 1: Base Types (~8 types)
**Source:** `base-schemas.ts`
- `WriteConcern` enum: FIRE_AND_FORGET, MEMORY, APPLIED, REPLICATED, PERSISTED
- `Timestamp` struct (modify existing in hlc.rs)
- `LWWRecord<V>` struct (modify existing in hlc.rs)
- `ORMapRecord<V>` struct (modify existing in hlc.rs)
- `PredicateOp` enum: eq, neq, gt, gte, lt, lte, like, regex, and, or, not
- `PredicateNode` struct (recursive, uses `Box<Vec<PredicateNode>>` for children)
- `Query` struct
- `ClientOp` struct
- `AuthMessage` struct (type = "AUTH", includes optional `protocolVersion`)

#### Domain 2: Sync Messages (~16 types)
**Source:** `sync-schemas.ts`
- `ClientOpMessage` (type = "CLIENT_OP")
- `OpBatchMessage` (type = "OP_BATCH"), `OpBatchPayload`
- `SyncInitMessage` (type = "SYNC_INIT")
- `SyncRespRootMessage` (type = "SYNC_RESP_ROOT"), `SyncRespRootPayload`
- `SyncRespBucketsMessage` (type = "SYNC_RESP_BUCKETS"), `SyncRespBucketsPayload`
- `SyncRespLeafMessage` (type = "SYNC_RESP_LEAF"), `SyncRespLeafPayload`, `SyncLeafRecord`
- `MerkleReqBucketMessage` (type = "MERKLE_REQ_BUCKET"), `MerkleReqBucketPayload`
- 8 ORMap sync variants: ORMAP_SYNC_INIT, ORMAP_SYNC_RESP_ROOT, ORMAP_SYNC_RESP_BUCKETS, ORMAP_MERKLE_REQ_BUCKET, ORMAP_SYNC_RESP_LEAF, ORMAP_DIFF_REQUEST, ORMAP_DIFF_RESPONSE, ORMAP_PUSH_DIFF
- `ORMapLeafEntry` (shared sub-type: key + records + tombstones)
- `OpResult`, `OpAckMessage` (type = "OP_ACK"), `OpRejectedMessage` (type = "OP_REJECTED")
- `BatchMessage` (type = "BATCH") -- binary payload with `Uint8Array` mapped to `Vec<u8>`

#### Domain 3: Query Messages (~4 types)
**Source:** `query-schemas.ts`
- `QuerySubMessage` (type = "QUERY_SUB"), `QuerySubPayload`
- `QueryUnsubMessage` (type = "QUERY_UNSUB"), `QueryUnsubPayload`
- `CursorStatus` enum: valid, expired, invalid, none
- `QueryRespMessage` (type = "QUERY_RESP"), `QueryRespPayload`, `QueryResultEntry`

#### Domain 4: Search Messages (~6 types)
**Source:** `search-schemas.ts`
- `SearchOptions`
- `SearchMessage` (type = "SEARCH"), `SearchPayload`
- `SearchRespMessage` (type = "SEARCH_RESP"), `SearchRespPayload`, `SearchResultEntry`
- `SearchUpdateType` enum: ENTER, UPDATE, LEAVE
- `SearchSubMessage` (type = "SEARCH_SUB"), `SearchSubPayload`
- `SearchUpdateMessage` (type = "SEARCH_UPDATE"), `SearchUpdatePayload`
- `SearchUnsubMessage` (type = "SEARCH_UNSUB"), `SearchUnsubPayload`

#### Domain 5: Cluster Messages (~10 types)
**Source:** `cluster-schemas.ts`
- `PartitionMapRequestMessage` (type = "PARTITION_MAP_REQUEST")
- `ClusterSubRegisterMessage` (type = "CLUSTER_SUB_REGISTER"), `ClusterSubRegisterPayload`
- `ClusterSubAckMessage` (type = "CLUSTER_SUB_ACK"), `ClusterSubAckPayload`, `ClusterSubAckResult`
- `ClusterSubUpdateMessage` (type = "CLUSTER_SUB_UPDATE"), `ClusterSubUpdatePayload`
- `ClusterSubUnregisterMessage` (type = "CLUSTER_SUB_UNREGISTER")
- `ClusterSearchReqMessage` (type = "CLUSTER_SEARCH_REQ"), `ClusterSearchReqPayload`, `ClusterSearchOptions`
- `ClusterSearchRespMessage` (type = "CLUSTER_SEARCH_RESP"), `ClusterSearchRespPayload`, `ClusterSearchResult`
- `ClusterSearchSubscribeMessage` (type = "CLUSTER_SEARCH_SUBSCRIBE")
- `ClusterSearchUnsubscribeMessage` (type = "CLUSTER_SEARCH_UNSUBSCRIBE")
- `ClusterSearchUpdateMessage` (type = "CLUSTER_SEARCH_UPDATE"), `ClusterSearchUpdatePayload`

#### Domain 6: Messaging Messages (~20 types)
**Source:** `messaging-schemas.ts`
- Topic: `TopicSubMessage`, `TopicUnsubMessage`, `TopicPubMessage`, `TopicMessageEvent`
- Lock: `LockRequestMessage`, `LockReleaseMessage`
- Counter: `PNCounterState`, `CounterRequestMessage`, `CounterSyncMessage`, `CounterResponseMessage`, `CounterUpdateMessage`
- Heartbeat: `PingMessage`, `PongMessage`
- EntryProcessor: `EntryProcessor`, `EntryProcessRequest`, `EntryProcessBatchRequest`, `EntryProcessResponse`, `EntryProcessKeyResult`, `EntryProcessBatchResponse`
- Journal: `JournalEventType`, `JournalEventData`, `JournalSubscribeRequest`, `JournalUnsubscribeRequest`, `JournalEventMessage`, `JournalReadRequest`, `JournalReadResponse`
- ConflictResolver: `ConflictResolver`, `RegisterResolverRequest/Response`, `UnregisterResolverRequest/Response`, `MergeRejectedMessage`, `ListResolversRequest/Response`, `ResolverInfo`

#### Domain 7: Client Event Messages (~10 types)
**Source:** `client-message-schemas.ts`
- `AuthAckMessage` (type = "AUTH_ACK"), includes optional `protocolVersion`
- `AuthFailMessage` (type = "AUTH_FAIL")
- `ErrorMessage` (type = "ERROR"), payload: `{ code, message, details? }`
- `ServerEventPayload`, `ServerEventMessage` (type = "SERVER_EVENT")
- `ServerBatchEventMessage` (type = "SERVER_BATCH_EVENT")
- `QueryUpdateMessage` (type = "QUERY_UPDATE"), `QueryUpdatePayload`
- `GcPruneMessage` (type = "GC_PRUNE"), `GcPrunePayload`
- `LockGrantedPayload` (includes `name`), `LockReleasedPayload` (includes `name`)
- `SyncResetRequiredPayload`

**Removed (dead code):** `HybridQueryRespPayload`, `HybridQueryDeltaPayload` — deleted in protocol cleanup.

#### Domain 8: HTTP Sync Types (~10 types)
**Source:** `http-sync-schemas.ts`
- `SyncMapEntry`, `HttpQueryRequest`, `HttpSearchRequest`
- `HttpSyncRequest`
- `DeltaRecord`, `MapDelta`
- `HttpQueryResult`, `HttpSearchResult`, `HttpSyncError`
- `HttpSyncResponse`

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/mod.rs` | Module declarations, `Message` enum (internally tagged), re-exports |
| `packages/core-rust/src/messages/base.rs` | `WriteConcern`, `PredicateOp`, `PredicateNode`, `Query`, `ClientOp`, `AuthMessage` |
| `packages/core-rust/src/messages/sync.rs` | All sync-domain structs and enums |
| `packages/core-rust/src/messages/query.rs` | Query sub/unsub/resp structs |
| `packages/core-rust/src/messages/search.rs` | Search structs and `SearchUpdateType` |
| `packages/core-rust/src/messages/cluster.rs` | Cluster sub/search structs |
| `packages/core-rust/src/messages/messaging.rs` | Topic, lock, counter, ping/pong, entry processor, journal, conflict resolver structs |
| `packages/core-rust/src/messages/client_events.rs` | Server-to-client event structs |
| `packages/core-rust/src/messages/http_sync.rs` | HTTP sync request/response structs |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/hlc.rs` | Add `#[serde(rename_all = "camelCase")]` to `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` |
| `packages/core-rust/src/lib.rs` | Add `pub mod messages;` and re-exports |
| `packages/core-rust/Cargo.toml` | Add `rmpv = "1"` dependency |

### Files to Create (Tests)

| File | Contents |
|------|----------|
| `packages/core-rust/tests/cross_lang_compat.rs` | Integration tests reading golden fixtures, verifying decode/re-encode |
| `packages/core/src/__tests__/cross-lang-fixtures.test.ts` | TS fixture generator writing golden MsgPack + JSON files |
| `packages/core-rust/tests/fixtures/` | Directory for golden MsgPack fixture files (generated by TS, read by Rust) |

### Serde Patterns

**Struct pattern:**
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInitMessage {
    pub map_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_sync_timestamp: Option<f64>,
}
```

**Discriminated union pattern (top-level):**
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    #[serde(rename = "AUTH")]
    Auth(AuthMessage),
    #[serde(rename = "CLIENT_OP")]
    ClientOp(ClientOpMessage),
    // ... all 44+ variants
}
```

**Nested `type` field conflict pattern:**
```rust
// When a payload struct has a `type` field that conflicts with the discriminant:
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchUpdatePayload {
    pub subscription_id: String,
    pub key: String,
    pub value: rmpv::Value,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub matched_terms: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub update_type: SearchUpdateType,
}
```

**Dynamic value pattern:**
```rust
// For z.any() / z.unknown() fields:
pub value: rmpv::Value,
// For z.any().nullable():
pub value: Option<rmpv::Value>,
```

## Acceptance Criteria

1. **AC-1: All TS MessageSchema variants decode in Rust.** Every one of the 44 variants in the TS `MessageSchema` discriminated union can be `msgpackr.pack()`-ed by TS and `rmp_serde::from_slice::<Message>()`-ed by Rust without error.

2. **AC-2: All additional message types decode in Rust.** The ~15 message types NOT in the union (ServerEvent, ClusterSearch, HTTP sync, etc.) each have a dedicated Rust struct that decodes MsgPack produced by TS.

3. **AC-3: Rust re-encode matches TS decode.** For every message type: Rust `rmp_serde::to_vec_named()` output can be `msgpackr.unpack()`-ed by TS and passes the corresponding Zod schema validation.

4. **AC-4: Round-trip fidelity.** TS encode -> Rust decode -> Rust re-encode -> TS decode produces semantically identical data for all message types. Verified by golden-file integration tests.

5. **AC-5: Optional field omission.** When a Rust struct has `None` for an optional field, the serialized MsgPack does NOT contain that key. Verified by byte inspection in at least 3 representative message types.

6. **AC-6: Existing Rust types compatible.** `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` with `#[serde(rename_all = "camelCase")]` produce MsgPack (via `to_vec_named()`) that TS can decode. Existing Rust-only tests using `rmp_serde::to_vec()` (array format) continue to pass.

7. **AC-7: Discriminated union works.** The `Message` enum with `#[serde(tag = "type")]` correctly routes deserialization based on the `type` field string value for all variants.

8. **AC-8: cargo test passes.** All existing core-rust tests pass. All new message serde tests pass. No regressions.

9. **AC-9: Golden fixture coverage.** At least one golden fixture file exists per schema domain (8 domains), covering at minimum 30 distinct message types total.

## Constraints

- Do NOT implement message handler logic -- this spec is strictly struct definitions and serde configuration.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT use `rmp_serde::to_vec()` (array format) for wire messages -- always use `rmp_serde::to_vec_named()` for cross-language messages.
- Do NOT replace the existing `Value` enum in `types.rs` -- use `rmpv::Value` for wire-boundary dynamic values; `crate::Value` remains for typed CRDT operations.
- Do NOT add `rmpv` dependency to anything outside `core-rust`.
- Max 5 Rust files modified/created per sub-spec after splitting.

## Assumptions

- **msgpackr default behavior is sufficient:** No special msgpackr configuration (structures, bundleStrings, etc.) is used by the TS serializer. Confirmed by reading `serializer.ts`.
- **Payload structs are flattened into message structs:** Where TS has `{ type: "SYNC_RESP_ROOT", payload: { mapName, rootHash, timestamp } }`, Rust uses `#[serde(rename = "payload")]` on a nested struct field, NOT `#[serde(flatten)]`. This matches the wire format exactly.
- **`z.number()` maps to `f64` in Rust** for most cases (JS numbers are IEEE 754 doubles). Integer-specific fields (counters, counts, codes) use appropriate integer types with serde handling.
- **rmpv 1.x is compatible** with the rmp-serde 1.x already in use.
- **Fixture files are checked into git** in a small `tests/fixtures/` directory within core-rust. The TS fixture generator test creates them; Rust reads them.
- **BATCH message's `data` field** (Uint8Array in TS) maps to `Vec<u8>` in Rust with MsgPack bin format -- this is the default for both msgpackr and rmp-serde.
- **Client-event payload types** (LockGrantedPayload, LockReleasedPayload, ErrorMessage, AuthAckMessage, etc.) that are not in the MessageSchema union are still needed as standalone Rust structs for server-to-client message construction.

## Goal-Backward Analysis

### Goal Statement
The Rust server can participate in the TopGun wire protocol, correctly reading messages from TS clients and sending messages that TS clients can read, for the complete message vocabulary.

### Required Wiring
- `messages/mod.rs` -> `messages/base.rs`: Base types used by all other domain modules
- `messages/sync.rs` -> `messages/base.rs`: Uses `ClientOp`, `LWWRecord`, `ORMapRecord`, `Timestamp`, `WriteConcern`
- `messages/query.rs` -> `messages/base.rs`: Uses `Query`
- `messages/search.rs` -> (self-contained enums)
- `messages/cluster.rs` -> `messages/search.rs`: Uses `SearchUpdateType`, `SearchOptions`
- `messages/messaging.rs` -> `messages/base.rs`: Uses `Timestamp`
- `messages/client_events.rs` -> `messages/base.rs`, `messages/query.rs`, `messages/search.rs`: Uses `LWWRecord`, `ORMapRecord`, `CursorStatus`, `Timestamp`
- `messages/http_sync.rs` -> `messages/base.rs`, `messages/sync.rs`: Uses `Timestamp`, `LWWRecord`, `ClientOp`, `OpResult`
- `hlc.rs` change -> ALL existing Rust code: serde rename must not break array-format serialization

### Key Risk
The `hlc.rs` serde rename is the highest-risk change. Adding `rename_all = "camelCase"` affects `to_vec_named()` output but NOT `to_vec()` (array format). Since existing tests use `to_vec()`, they should be unaffected. But any code path that uses `to_vec_named()` for Timestamp today (there appear to be none) would change behavior.

## Implementation Tasks

### Splitting Required

This spec defines **9 new Rust source files + 2 modified files + 2 test files = 13 files total**. The Rust Language Profile mandates max 5 files per spec. This spec **MUST be split** via `/sf:split` before implementation.

### Recommended Split Strategy

| Sub-Spec | Domain | New Files | Modified Files | Description |
|-----------|--------|-----------|----------------|-------------|
| SPEC-052a | Foundation | 2 | 3 | `messages/mod.rs`, `messages/base.rs` + modify `hlc.rs`, `lib.rs`, `Cargo.toml`. Serde rename on existing types, base enums/structs, module scaffold. |
| SPEC-052b | Sync + Query | 2 | 0 | `messages/sync.rs`, `messages/query.rs`. All LWW/ORMap sync messages, query sub/unsub/resp. |
| SPEC-052c | Search + Cluster | 2 | 0 | `messages/search.rs`, `messages/cluster.rs`. Search and cluster domain messages. |
| SPEC-052d | Messaging + Client Events | 2 | 0 | `messages/messaging.rs`, `messages/client_events.rs`. Topics, locks, counters, journal, processors, resolvers, server events. |
| SPEC-052e | HTTP Sync + Union + Cross-Lang Tests | 1 + tests | 1 | `messages/http_sync.rs`, update `messages/mod.rs` with full `Message` enum, golden fixture tests (TS generator + Rust verifier). |

### Task Groups (Pre-Split Overview)

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Foundation: serde renames on existing types, `messages/mod.rs` scaffold, `messages/base.rs` with shared types/enums, `rmpv` dep | -- | ~20% |
| G2 | 2 | Sync domain: `messages/sync.rs` with all LWW/ORMap sync structs | G1 | ~20% |
| G3 | 2 | Query domain: `messages/query.rs` | G1 | ~5% |
| G4 | 2 | Search domain: `messages/search.rs` | G1 | ~10% |
| G5 | 3 | Cluster domain: `messages/cluster.rs` | G1, G4 | ~10% |
| G6 | 2 | Messaging domain: `messages/messaging.rs` | G1 | ~15% |
| G7 | 3 | Client events: `messages/client_events.rs` | G1, G3, G4 | ~10% |
| G8 | 3 | HTTP sync: `messages/http_sync.rs` | G1, G2 | ~5% |
| G9 | 4 | Full `Message` enum in mod.rs, golden fixtures (TS + Rust), integration tests | G1-G8 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3, G4, G6 | Yes | 4 |
| 3 | G5, G7, G8 | Yes | 3 |
| 4 | G9 | No | 1 |

**Total workers needed:** 4 (max in any wave)

---
*Generated by SpecFlow on 2026-02-14. Updated 2026-02-14 to reflect protocol cleanup (commit 53b60e4). This spec requires `/sf:split` before implementation.*
