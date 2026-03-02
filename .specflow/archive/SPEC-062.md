# SPEC-062: Implement CrdtService (LWW-Map and OR-Map Operations)

```yaml
id: SPEC-062
type: feature
status: done
priority: P0
complexity: medium
created: 2026-02-24
todo: TODO-085
```

## Context

The CrdtService is the core data path for the Rust server. After CoordinationService (SPEC-061) validated the full message pipeline (WS -> deserialize -> classify -> route -> domain service -> serialize -> WS response), CrdtService implements the actual data mutations that make TopGun useful as a data grid.

Currently, `CrdtService` is a `domain_stub!` macro that returns `OperationResponse::NotImplemented` for all operations. This spec replaces that stub with a real implementation that:

1. Receives `ClientOp` and `OpBatch` operations from the routing pipeline
2. Merges LWW and OR-Map data into the `RecordStore`
3. Broadcasts `ServerEvent` messages to subscribed client connections
4. Returns `OpAck` responses to the caller

**TS behavioral reference:** `packages/server/src/coordinator/operation-handler.ts` (`OperationHandler.processLocalOp` and `applyOpToMap`).

**Established pattern:** SPEC-061 `CoordinationService` (`packages/server-rust/src/service/domain/coordination.rs`) demonstrates how to replace a `domain_stub!` with a real `tower::Service<Operation>` implementation.

## Goal Analysis

**Goal Statement:** When a client sends a `ClientOp` or `OpBatch` message, the server merges the CRDT data into storage and broadcasts the change to all connected clients.

**Observable Truths:**

1. A `ClientOp` with an LWW record is merged into `RecordStore` via `put()` with `CallerProvenance::CrdtMerge`, and a `ServerEvent(PUT)` is broadcast to all client connections.
2. A `ClientOp` with `record: Some(None)` (tombstone/null value) is treated as a REMOVE and a `ServerEvent(REMOVE)` is broadcast.
3. A `ClientOp` with an OR-Map record (`or_record`) triggers an OR_ADD merge and a `ServerEvent(OR_ADD)` is broadcast.
4. A `ClientOp` with an `or_tag` (and no `or_record`) triggers an OR_REMOVE and a `ServerEvent(OR_REMOVE)` is broadcast.
5. An `OpBatch` processes each operation sequentially and returns a single `OpAck` with the last operation's ID.
6. Non-CRDT operations routed to CrdtService return `OperationError::WrongService`.
7. `CrdtService` integrates into the existing `ServiceRegistry` and `OperationRouter` without changing their APIs.

**Required Artifacts:**

| Artifact | Purpose |
|----------|---------|
| `packages/server-rust/src/service/domain/crdt.rs` | CrdtService struct, handler methods, tower::Service impl, tests |
| `packages/server-rust/src/service/domain/mod.rs` | Remove CrdtService stub, add `pub mod crdt` and re-export |
| `packages/server-rust/src/lib.rs` | Wire CrdtService with RecordStoreFactory + ConnectionRegistry in integration tests |

**Key Links:**

- `CrdtService` -> `RecordStoreFactory` (creates RecordStores per map/partition)
- `CrdtService` -> `ConnectionRegistry` (broadcast ServerEvent to clients)
- `Operation::ClientOp` -> `ClientOpMessage` -> `ClientOp` (message payload chain)
- `RecordStore::put()` accepts `RecordValue` + `CallerProvenance::CrdtMerge`
- `ConnectionRegistry::broadcast()` sends binary to all connections of a kind

## Task

Replace the `domain_stub!(CrdtService, ...)` macro in `packages/server-rust/src/service/domain/mod.rs` with a real `CrdtService` struct in a new `crdt.rs` module that:

1. Takes `Arc<RecordStoreFactory>` and `Arc<ConnectionRegistry>` as constructor dependencies
2. Implements `ManagedService` for lifecycle management
3. Implements `tower::Service<Operation>` for `Arc<CrdtService>` to handle `ClientOp` and `OpBatch` variants
4. Converts `ClientOp` payloads into `RecordValue` and calls `RecordStore::put()`/`RecordStore::remove()`
5. Constructs `ServerEventPayload` from the operation data and broadcasts via `ConnectionRegistry`
6. Returns `OpAck` or `OpRejected` responses as appropriate

## Requirements

### Files to Create

**1. `packages/server-rust/src/service/domain/crdt.rs`**

**Key `use` imports required in `crdt.rs`:**

```rust
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use topgun_core::messages::{
    ClientOp, ClientOpMessage, Message, OpAckMessage, OpAckPayload,
    OpBatchMessage, ServerEventPayload, ServerEventType,
};
use topgun_core::types::Value;
use topgun_core::{LWWRecord, ORMapRecord};

use crate::network::connection::{ConnectionKind, ConnectionRegistry};
use crate::service::operation::{
    service_names, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::record::{OrMapEntry, RecordValue};
use crate::storage::{CallerProvenance, ExpiryPolicy, RecordStoreFactory};
```

**Struct definition:**

```rust
pub struct CrdtService {
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
}
```

**Constructor:** `CrdtService::new(record_store_factory, connection_registry)`

**ManagedService impl:** Same pattern as `CoordinationService` -- `name()` returns `service_names::CRDT`, `init`/`reset`/`shutdown` are no-ops for now.

**tower::Service<Operation> for Arc<CrdtService>:**

Match on operation variants:
- `Operation::ClientOp { ctx, payload }` -> `handle_client_op(&ctx, payload)`
- `Operation::OpBatch { ctx, payload }` -> `handle_op_batch(&ctx, payload)`
- Any other variant -> `Err(OperationError::WrongService)`

**`handle_client_op` logic:**

1. Extract `map_name`, `key`, `op_type` from `payload.payload` (which is a `ClientOp`)
2. Determine operation type:
   - If `op_type == Some("REMOVE")` OR `record == Some(None)` (tombstone): this is a REMOVE
   - If `or_record` is `Some(Some(_))`: this is an OR_ADD
   - If `or_tag` is `Some(Some(_))` and `or_record` is None/absent: this is an OR_REMOVE
   - Otherwise: this is a PUT (LWW)
3. Create a `RecordStore` via `record_store_factory.create(map_name, partition_id)` where `partition_id` comes from `ctx.partition_id.unwrap_or(0)`
4. For LWW PUT: Convert `LWWRecord<rmpv::Value>` into `RecordValue::Lww` and call `record_store.put(key, value, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)`
5. For LWW REMOVE: Call `record_store.remove(key, CallerProvenance::CrdtMerge)`
6. For OR_ADD: Convert to `RecordValue::OrMap` and call `record_store.put(...)`
7. For OR_REMOVE: Build `RecordValue::OrTombstones { tags: vec![tag.clone()] }` and call `record_store.put(key, value, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)`
8. Build a `ServerEventPayload` with the appropriate `ServerEventType`, `map_name`, `key`, `record`/`or_record`/`or_tag`
9. Serialize `Message::ServerEvent { payload }` to MsgPack bytes via `rmp_serde::to_vec_named()`
10. Call `connection_registry.broadcast(&bytes, ConnectionKind::Client)` to push the event to all connected clients
11. Return `OperationResponse::Message(Box::new(Message::OpAck(...)))` with the operation ID (or `OperationResponse::Ack` if no ID)

**`handle_op_batch` logic:**

1. Iterate over `payload.payload.ops` (Vec<ClientOp>)
2. For each op, apply the same logic as `handle_client_op` (extract into a helper `apply_single_op`)
3. After all ops processed, return `OperationResponse::Message(Box::new(Message::OpAck(OpAckMessage { payload: OpAckPayload { last_id, ... } })))` where `last_id` is the ID of the last operation in the batch

**Helper: `apply_single_op`**

Shared between `handle_client_op` and `handle_op_batch`:
```rust
async fn apply_single_op(
    &self,
    op: &ClientOp,
    partition_id: u32,
) -> Result<ServerEventPayload, OperationError>
```

Returns the `ServerEventPayload` for broadcast. The caller decides whether to broadcast individually (ClientOp) or batch (OpBatch).

**Conversion helpers:**

- `rmpv_to_value(v: &rmpv::Value) -> topgun_core::types::Value`: Recursive conversion from wire format `rmpv::Value` to storage `topgun_core::types::Value`. Maps: Null->Null, Boolean->Bool, Integer->Int(i64), Float->Float(f64), String->String, Binary->Bytes, Array->Array(recursive), Map->Map(BTreeMap, recursive).

  **Integer handling:** `rmpv::Value::Integer` wraps `rmpv::Integer` which can be signed (`i64`) or unsigned (`u64`). Try `v.as_i64()` first; if that returns `None` (value exceeds i64 max), fall back to `v.as_u64().unwrap_or(0) as i64`. This maps to `topgun_core::types::Value::Int(i64)`.

  **Map key handling:** `rmpv::Value::Map` uses `Vec<(rmpv::Value, rmpv::Value)>`. Coerce each key to a `String` (use `key.to_string()`) and recursively convert values, collecting into `BTreeMap<String, Value>`.

  This function is required because `topgun_core::types::Value` is a custom enum (Null, Bool, Int, Float, String, Bytes, Array, Map) with no `From<rmpv::Value>` impl.

- `lww_record_to_record_value(record: &LWWRecord<rmpv::Value>) -> RecordValue`: Converts wire LWW record to storage `RecordValue::Lww` using `rmpv_to_value` for the inner value.
- `or_record_to_record_value(record: &ORMapRecord<rmpv::Value>) -> RecordValue`: Wraps a single OR-Map record into `RecordValue::OrMap { records: vec![OrMapEntry { ... }] }`, using `rmpv_to_value` for the inner value.

### Files to Modify

**2. `packages/server-rust/src/service/domain/mod.rs`**

- Remove `domain_stub!(CrdtService, service_names::CRDT);`
- Add `pub mod crdt;`
- Add `pub use crdt::CrdtService;`
- Remove the `crdt_service_returns_not_implemented` test (CrdtService is no longer a stub)
- Update the `all_stubs_implement_managed_service` test: remove `registry.register(CrdtService)` and remove `assert!(registry.get_by_name("crdt").is_some())` (CrdtService now requires constructor args, like CoordinationService)

**3. `packages/server-rust/src/lib.rs`** (integration tests only)

- Update `setup()`: Replace `Arc::new(CrdtService)` (line 72) with `Arc::new(CrdtService::new(record_store_factory.clone(), connection_registry.clone()))`, creating a `RecordStoreFactory` with `NullDataStore` and empty observers. The `connection_registry` variable already exists in `setup()` (line 69) and can be reused.
- Rename `full_pipeline_client_op_to_not_implemented` (line 121) to `full_pipeline_client_op_to_op_ack` and update its assertion from `OperationResponse::NotImplemented { service_name: "crdt", .. }` to `OperationResponse::Message(msg) if matches!(*msg, Message::OpAck(_))`.
- Update `service_registry_lifecycle` (line 210): Replace `registry.register(CrdtService)` (line 221) with `registry.register(CrdtService::new(record_store_factory, connection_registry_for_crdt))` where `record_store_factory` uses `NullDataStore` and `connection_registry_for_crdt` is a freshly created `Arc::new(ConnectionRegistry::new())`. Also update `registry.get::<CrdtService>().is_some()` assertion -- this remains unchanged since the type still exists; only the registration call changes.

### Value Conversion Note

The `ClientOp.record` field is `Option<Option<LWWRecord<rmpv::Value>>>` and `RecordValue::Lww` uses `topgun_core::types::Value`. These are **different types**: `rmpv::Value` is a dynamic MsgPack value, while `topgun_core::types::Value` is a custom enum (Null, Bool, Int, Float, String, Bytes, Array, Map). No `From` conversion exists. A recursive `rmpv_to_value` helper must be implemented in `crdt.rs`.

## Acceptance Criteria

### AC1: LWW PUT -- ClientOp with record merges into RecordStore

Given a `ClientOp` with `map_name: "users"`, `key: "user-1"`, and `record: Some(Some(LWWRecord { value: Some(...), ... }))`:
- `RecordStore::put()` is called with `CallerProvenance::CrdtMerge`
- `ServerEvent { payload: ServerEventPayload { event_type: PUT, map_name: "users", key: "user-1", record: Some(...) } }` is broadcast
- Response is `OperationResponse::Message(Message::OpAck(...))`

### AC2: LWW REMOVE -- ClientOp with tombstone record merges as remove

Given a `ClientOp` with `record: Some(None)` or `op_type: Some("REMOVE")`:
- `RecordStore::remove()` is called with `CallerProvenance::CrdtMerge`
- `ServerEvent { payload: ServerEventPayload { event_type: REMOVE, ... } }` is broadcast
- Response is `OperationResponse::Message(Message::OpAck(...))`

### AC3: OR_ADD -- ClientOp with or_record adds to OR-Map

Given a `ClientOp` with `or_record: Some(Some(ORMapRecord { ... }))`:
- `RecordStore::put()` is called with a `RecordValue::OrMap` containing the new entry
- `ServerEvent { payload: ServerEventPayload { event_type: OR_ADD, or_record: Some(...) } }` is broadcast
- Response is `OperationResponse::Message(Message::OpAck(...))`

### AC4: OR_REMOVE -- ClientOp with or_tag removes from OR-Map

Given a `ClientOp` with `or_tag: Some(Some("tag-1"))` and no `or_record`:
- `RecordStore::put()` is called with `RecordValue::OrTombstones { tags: vec!["tag-1".to_string()] }` and `CallerProvenance::CrdtMerge`
- `ServerEvent { payload: ServerEventPayload { event_type: OR_REMOVE, or_tag: Some("tag-1") } }` is broadcast
- Response is `OperationResponse::Message(Message::OpAck(...))`

### AC5: OpBatch processes all operations and returns single OpAck

Given an `OpBatch` with 3 `ClientOp` entries (IDs "op-1", "op-2", "op-3"):
- All 3 operations are processed (RecordStore calls made for each)
- A single `OpAck` response is returned with `last_id: "op-3"`
- ServerEvent is broadcast for each individual operation

### AC6: Wrong service returns WrongService error

Given any non-CRDT `Operation` variant (e.g., `Operation::Ping`, `Operation::GarbageCollect`):
- Returns `Err(OperationError::WrongService)`

### AC7: ManagedService name is "crdt"

`CrdtService::new(...).name()` returns `"crdt"`.

### AC8: Integration test -- CrdtService replaces stub in full pipeline

The existing integration test `setup()` in `lib.rs` creates a real `CrdtService` with `RecordStoreFactory(NullDataStore)` and `ConnectionRegistry`. A `ClientOp` message routed through the pipeline returns an `OpAck` (not `NotImplemented`). The test previously named `full_pipeline_client_op_to_not_implemented` is renamed to `full_pipeline_client_op_to_op_ack` with the assertion updated accordingly.

### AC9: OpBatch with empty ops returns OpAck with empty last_id

Given an `OpBatch` with zero operations, returns `OperationResponse::Ack` or `OperationResponse::Empty` (no operations to acknowledge).

## Constraints

- Do NOT add interceptor/before-after hooks (TS has these but they are out of scope for this spec)
- Do NOT implement partition routing/forwarding (the TS `processClientOp` checks `isLocalOwner` and forwards -- defer to a future spec)
- Do NOT implement conflict resolver integration (TS has `conflictResolverHandler` -- defer to PersistenceService)
- Do NOT implement replication pipeline integration (defer to cluster integration)
- Do NOT implement Merkle tree updates (defer to SyncService)
- Do NOT implement search index updates (defer to SearchService)
- Do NOT implement event journal writes (defer to PersistenceService)
- Do NOT add permission checks (defer to auth/security layer)
- Do NOT implement Write Concern tracking (defer to a future spec)
- All data goes through `RecordStore` -- do NOT create in-memory LWWMap/ORMap instances directly
- Follow the `CoordinationService` pattern exactly for struct layout, Arc wrapping, and test helpers

## Assumptions

- `topgun_core::types::Value` is a custom enum (Null, Bool, Int, Float, String, Bytes, Array, Map) distinct from `rmpv::Value`. No `From<rmpv::Value>` conversion exists. A recursive `rmpv_to_value` helper must be implemented in `crdt.rs`. See the Integer and Map key handling notes in the conversion helpers section above.
- `RecordStoreFactory::create()` can be called per-operation to get a `Box<dyn RecordStore>` for the given map/partition. In a production deployment, stores would be cached, but for this spec calling `create()` each time is acceptable.
- `ConnectionRegistry::broadcast()` is the appropriate mechanism for pushing ServerEvent to clients. The TS implementation has more sophisticated subscriber filtering (per-map subscription tracking), but for this spec, broadcasting to all clients is the correct first step. Sender exclusion is NOT implemented in this spec -- all clients receive the broadcast.
- `partition_id` from `OperationContext` is used as-is. If `None`, default to `0`. Partition routing is a separate concern.
- The `OpAck` response uses the `id` field from the last `ClientOp` in a batch, or `"unknown"` if no ID is present.
- `NullDataStore` (from storage::datastores) is sufficient for tests -- it discards writes but allows put/get round-trips through the in-memory engine layer.
- `RecordValue::OrTombstones` exists in `packages/server-rust/src/storage/record.rs` with field `tags: Vec<String>`. This is the correct variant for OR_REMOVE operations (confirmed by code inspection).

## Implementation Tasks

### Task Groups

| Group | Wave | Segment | Tasks | Dependencies | Est. Context |
|-------|------|---------|-------|--------------|--------------|
| G1 | 1 | -- | Define `CrdtService` struct with constructor and `ManagedService` impl (no dispatch logic) | -- | ~15% |
| G2 | 2 | S1 | Implement conversion helpers: `rmpv_to_value`, `lww_record_to_record_value`, `or_record_to_record_value`. Implement `apply_single_op` skeleton with LWW PUT/REMOVE branches (RecordStore calls + ServerEventPayload construction) | G1 | ~25% |
| G2 | 2 | S2 | Complete `apply_single_op` with OR_ADD/OR_REMOVE branches. Implement MsgPack serialization and `ConnectionRegistry::broadcast()`. Implement `handle_client_op` using `apply_single_op`. Wire `tower::Service<Operation>` dispatch (match arms for ClientOp/OpBatch/WrongService). | G2-S1 | ~25% |
| G3 | 2 | -- | Implement `handle_op_batch` iterating ops and collecting `OpAck`. | G2-S1 | ~15% |
| G4 | 3 | S1 | Update `mod.rs`: remove stub, add `pub mod crdt`, re-export, remove `crdt_service_returns_not_implemented` test, update `all_stubs_implement_managed_service`. Update `lib.rs`: update `setup()` to use real `CrdtService`, rename and update `full_pipeline_client_op_to_not_implemented` test, update `service_registry_lifecycle` registration. | G2-S2, G3 | ~20% |
| G4 | 3 | S2 | Write unit tests in `crdt.rs` for all 9 acceptance criteria (AC1-AC9). | G4-S1 | ~20% |

### Execution Plan

| Wave | Groups/Segments | Parallel? | Workers |
|------|-----------------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2-S1 | No (sequential within G2) | 1 |
| 2 | G2-S2, G3 | Yes (after G2-S1) | 2 |
| 3 | G4-S1 | No | 1 |
| 3 | G4-S2 | No (after G4-S1) | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-24 16:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (G1:15% + G2:30% + G3:20% + G4:35%)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- no new payload structs created)
- [x] Enums used for known value sets (uses existing `ServerEventType` enum)
- [x] Wire compatibility: uses `rmp_serde::to_vec_named()`
- [x] `#[serde(rename_all = "camelCase")]` (N/A -- no new serializable structs)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` (N/A)

**Critical:**

1. **Value type mismatch not resolved in spec.** The spec lists as an assumption that `topgun_core::types::Value` "is either `rmpv::Value` or has a `From<rmpv::Value>` conversion." Code inspection confirms this assumption is **false**: `topgun_core::types::Value` is a custom enum (`Null | Bool(bool) | Int(i64) | Float(f64) | String(String) | Bytes(Vec<u8>) | Array(Vec<Value>) | Map(BTreeMap<String, Value>)`) with no `From<rmpv::Value>` impl. The spec must specify a recursive `rmpv_to_value` conversion helper as a required artifact, including handling of `rmpv::Value::Integer` (which can be signed or unsigned and needs mapping to `i64`) and `rmpv::Value::Map` (which uses `Vec<(Value, Value)>` and needs conversion to `BTreeMap<String, Value>` with string key coercion). This is not a "verify during implementation" item -- it is core logic that affects the shape of `apply_single_op` and all AC1-AC4 tests.

2. **Trait-first violation: G1 does not contain only types/traits.** The Language Profile mandates "G1 (Wave 1) defines traits/types only, implementation groups depend on G1." The current G1 includes a full `tower::Service<Operation>` dispatch skeleton with match arms, which is implementation, not types. To comply, G1 should define only the struct, constructor, and `ManagedService` impl (which is effectively trait impl boilerplate). The dispatch skeleton should move to G2/G3.

3. **G2 estimated at ~30% exceeds per-group 30% warning threshold.** G2 includes: the `rmpv_to_value` conversion helper (non-trivial recursive function), `lww_record_to_record_value`, `apply_single_op` for LWW PUT/REMOVE, `ServerEventPayload` construction, MsgPack serialization, broadcast, and `handle_client_op`. This is too much logic for a single worker invocation. Should be split: S1 for conversion helpers + `apply_single_op`, S2 for broadcast + `handle_client_op` wiring.

4. **G4 estimated at ~35% exceeds per-group 30% warning threshold.** G4 includes updating two files (mod.rs, lib.rs) AND writing unit tests for all 9 acceptance criteria. Unit tests alone for 9 ACs in a Rust file with test setup helpers will be substantial. Should be split: S1 for mod.rs + lib.rs changes, S2 for unit tests.

5. **Missing `lib.rs` test update for `full_pipeline_client_op_to_not_implemented`.** The spec describes updating `setup()` and the `all_stubs_implement_managed_service` test, but does not mention that `full_pipeline_client_op_to_not_implemented` (line 121-159 of `lib.rs`) will now return `OpAck` instead of `NotImplemented`, breaking this test. The spec must specify renaming and updating this test assertion.

6. **Missing `service_registry_lifecycle` test update in `lib.rs`.** Line 221 of `lib.rs` calls `registry.register(CrdtService)` which is a unit struct. After this spec, `CrdtService` will require constructor args. The spec mentions this for `all_stubs_implement_managed_service` but not for `service_registry_lifecycle` (line 210-251).

**Recommendations:**

7. [Strategic] Observable Truth 1 says "excluding the sender" but the spec's implementation broadcasts to ALL clients via `ConnectionRegistry::broadcast()`. The constraint section does not mention sender exclusion. Either remove "excluding the sender" from Truth 1 to match actual behavior, or note this as a known simplification.

8. AC4 (OR_REMOVE) does not specify a `RecordStore` call. LWW REMOVE calls `record_store.remove()`, but OR_REMOVE with just a tag does not have an obvious `RecordStore` operation defined. The spec's step 6 says "For OR_ADD/OR_REMOVE: Convert to `RecordValue::OrMap` and call `record_store.put(...)`" but for OR_REMOVE, the appropriate storage operation would be `RecordValue::OrTombstones { tags: vec![tag] }` with `record_store.put()`, not `RecordValue::OrMap`. Clarify which `RecordValue` variant is used for OR_REMOVE and whether `put()` or `remove()` is called.

9. The `rmpv::Value::Integer` type wraps `rmpv::Integer` which can be `u64` or `i64`. The `rmpv_to_value` helper needs to handle both branches (try `as_i64()` first, fall back to `as_u64()` and cast). Document this edge case.

10. The spec could benefit from explicitly listing the `use` imports needed in `crdt.rs` to reduce implementer guesswork (e.g., `use topgun_core::messages::{ClientOp, ClientOpMessage, Message, OpAckMessage, OpAckPayload, OpBatchMessage, ServerEventPayload, ServerEventType}`).

### Response v1 (2026-02-24)
**Applied:** All 6 critical issues and all 4 recommendations

**Changes:**
1. [✓] Value type mismatch — Removed the incorrect "From<rmpv::Value> or alias" assumption. Updated Assumptions section to state explicitly that no From conversion exists. Updated conversion helpers section with full details of the recursive `rmpv_to_value` function. Added Integer and Map key handling to the description (see item 9 as well).
2. [✓] Trait-first violation — G1 now defines only struct, constructor, and `ManagedService` impl. The `tower::Service<Operation>` dispatch skeleton (match arms) moved to G2-S2. G1 description updated accordingly.
3. [✓] G2 ~30% too large — Split G2 into S1 (conversion helpers + apply_single_op LWW branches) at ~25% and S2 (OR branches + broadcast + handle_client_op + tower::Service dispatch) at ~25%. Each segment stays under the 30% threshold.
4. [✓] G4 ~35% too large — Split G4 into S1 (mod.rs + lib.rs changes) at ~20% and S2 (unit tests for AC1-AC9) at ~20%. Each segment stays under the 30% threshold.
5. [✓] Missing `full_pipeline_client_op_to_not_implemented` test update — Added to Files to Modify section for lib.rs: rename test to `full_pipeline_client_op_to_op_ack` and update assertion to match `OperationResponse::Message(msg) if matches!(*msg, Message::OpAck(_))`. Also added to AC8.
6. [✓] Missing `service_registry_lifecycle` test update — Added to Files to Modify section for lib.rs: replace `registry.register(CrdtService)` with `registry.register(CrdtService::new(record_store_factory, connection_registry_for_crdt))` with NullDataStore-backed factory.
7. [✓] Observable Truth 1 "excluding the sender" — Removed "excluding the sender" from Truth 1. Updated Assumptions to note that sender exclusion is NOT implemented and all clients receive the broadcast.
8. [✓] AC4 OR_REMOVE storage — Updated AC4 to specify `RecordValue::OrTombstones { tags: vec!["tag-1".to_string()] }` with `record_store.put()`. Updated `handle_client_op` step 7 to explicitly use `RecordValue::OrTombstones`. Added assumption confirming `OrTombstones` variant exists (confirmed by reading `packages/server-rust/src/storage/record.rs`).
9. [✓] rmpv::Value::Integer both branches — Added Integer handling note to the `rmpv_to_value` description: try `as_i64()` first, fall back to `as_u64().unwrap_or(0) as i64` for values exceeding i64 max.
10. [✓] Add `use` imports list for crdt.rs — Added a complete `use` imports block at the start of the Requirements > Files to Create section for `crdt.rs`.

### Audit v2 (2026-02-24 17:15)
**Status:** APPROVED

**Context Estimate:** ~25% per segment (max), 3 files total

**V1 Resolution Verification:**
- [x] Critical 1 (Value type mismatch): `rmpv_to_value` helper fully specified with Integer and Map key handling. `topgun_core::types::Value` confirmed as custom enum (Null, Bool, Int, Float, String, Bytes, Array, Map) -- matches code at `packages/core-rust/src/types.rs:27`.
- [x] Critical 2 (Trait-first): G1 now contains only struct + constructor + `ManagedService` impl. `tower::Service` dispatch moved to G2-S2.
- [x] Critical 3 (G2 too large): Split into G2-S1 (~25%) and G2-S2 (~25%), both under 30%.
- [x] Critical 4 (G4 too large): Split into G4-S1 (~20%) and G4-S2 (~20%), both under 30%.
- [x] Critical 5 (Missing `full_pipeline_client_op_to_not_implemented` update): Now in Files to Modify (lib.rs) and AC8.
- [x] Critical 6 (Missing `service_registry_lifecycle` update): Now in Files to Modify (lib.rs) with `NullDataStore` factory pattern.
- [x] Recommendation 7 (sender exclusion): "excluding the sender" removed from Truth 1.
- [x] Recommendation 8 (OR_REMOVE storage): AC4 specifies `RecordValue::OrTombstones` with `put()`.
- [x] Recommendation 9 (Integer handling): `as_i64()` -> `as_u64()` fallback documented.
- [x] Recommendation 10 (Import list): Full `use` block provided.

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- no new payload structs created)
- [x] Enums used for known value sets (uses existing `ServerEventType` enum)
- [x] Wire compatibility: uses `rmp_serde::to_vec_named()`
- [x] `#[serde(rename_all = "camelCase")]` (N/A -- no new serializable structs)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` (N/A)

**Dimension Assessment:**
- Clarity: All handler logic steps enumerated with exact field names and type paths. No vague terms.
- Completeness: All 3 files listed. Conversion helpers, handler methods, test updates, and import list all specified.
- Testability: 9 acceptance criteria, each with concrete Given/When/Then and specific type assertions.
- Scope: 12 "Do NOT" constraints clearly fence the work. No scope creep.
- Feasibility: Follows established CoordinationService pattern. All referenced APIs verified in source code.
- Architecture fit: tower::Service pattern, Arc wrapping, ManagedService -- all match existing codebase conventions.
- Non-duplication: No reinvention. Uses existing `RecordStoreFactory`, `ConnectionRegistry`, `RecordStore` trait.
- Cognitive load: Single new file with clear separation (conversion helpers, apply_single_op, handlers, dispatch). Follows existing pattern.
- Strategic fit: Core data path -- highest priority for making the Rust server functional.
- Project compliance: MsgPack wire format, trait-first grouping, 3 files within 5-file limit.
- Language profile: 3 files (under 5 max), G1 is types-only, segments under 30%.

**Goal-Backward Validation:**
| Check | Status | Notes |
|-------|--------|-------|
| Truth 1 (LWW PUT) has artifacts | OK | crdt.rs apply_single_op + RecordStore::put |
| Truth 2 (LWW REMOVE) has artifacts | OK | crdt.rs apply_single_op + RecordStore::remove |
| Truth 3 (OR_ADD) has artifacts | OK | crdt.rs apply_single_op + RecordValue::OrMap |
| Truth 4 (OR_REMOVE) has artifacts | OK | crdt.rs apply_single_op + RecordValue::OrTombstones |
| Truth 5 (OpBatch) has artifacts | OK | crdt.rs handle_op_batch |
| Truth 6 (WrongService) has artifacts | OK | crdt.rs tower::Service dispatch |
| Truth 7 (Integration) has artifacts | OK | mod.rs + lib.rs changes |
| CrdtService->RecordStoreFactory wiring | OK | Constructor dependency |
| CrdtService->ConnectionRegistry wiring | OK | Constructor dependency |

**Assumptions Validation:**
| # | Assumption | Verified |
|---|------------|----------|
| A1 | `topgun_core::types::Value` is custom enum, no `From<rmpv::Value>` | Confirmed: `types.rs:27` |
| A2 | `RecordStoreFactory::create()` returns `Box<dyn RecordStore>` | Confirmed: `factory.rs:48` |
| A3 | `ConnectionRegistry::broadcast()` takes `&[u8]` + `ConnectionKind` | Confirmed: `connection.rs:231` |
| A4 | `RecordValue::OrTombstones { tags: Vec<String> }` exists | Confirmed: `record.rs:97-100` |
| A5 | `CallerProvenance::CrdtMerge` exists | Confirmed: `record_store.rs:32` |
| A6 | `ExpiryPolicy::NONE` exists | Confirmed: `record_store.rs:49` |
| A7 | `OpAckPayload.last_id` is `String` | Confirmed: `sync.rs:426` |
| A8 | `NullDataStore` exists in `storage::datastores` | Confirmed: `datastores/null.rs` |

**Project Compliance:** Honors PROJECT.md decisions -- MsgPack wire format, trait-first task grouping, no new runtime dependencies, follows domain service replacement pattern.

**Strategic Fit:** Aligned with project goals -- CrdtService is the core data path, highest priority for Phase 3 server functionality.

**Recommendations:**

1. The import list is missing `use topgun_core::{LWWRecord, ORMapRecord};` -- these types are needed by the conversion helpers (`lww_record_to_record_value`, `or_record_to_record_value`) and by `apply_single_op` (extracting from `ClientOp.record` and `ClientOp.or_record`). They are re-exported from `topgun_core` via the `hlc` module, not from `topgun_core::messages`. Added in the spec file as part of this audit.

2. AC9 says "returns `OperationResponse::Ack` or `OperationResponse::Empty`" but `OperationResponse::Ack` requires `{ call_id: u64 }`. Consider clarifying which variant is preferred and what `call_id` to use (e.g., `ctx.call_id`). This is minor -- the implementer will make a reasonable choice.

**Comment:** Well-structured spec with thorough revision. All 6 critical issues from v1 have been addressed comprehensively. The spec is clear, complete, and implementable. The import list gap (LWWRecord/ORMapRecord) was fixed inline during this audit. Ready for implementation.

## Execution Summary

**Executed:** 2026-02-24
**Mode:** orchestrated
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2-S1, G2-S2, G3 | complete |
| 3 | G4-S1, G4-S2 | complete |

### Files Created

- `packages/server-rust/src/service/domain/crdt.rs` — CrdtService struct, ManagedService impl, tower::Service dispatch, apply_single_op, handle_client_op, handle_op_batch, rmpv_to_value, conversion helpers, unit tests for AC1-AC9

### Files Modified

- `packages/server-rust/src/service/domain/mod.rs` — Removed domain_stub!(CrdtService), added `pub mod crdt`, `pub use crdt::CrdtService`, updated all_stubs_implement_managed_service test
- `packages/server-rust/src/lib.rs` — Updated setup() to use real CrdtService::new(), renamed full_pipeline_client_op_to_not_implemented to full_pipeline_client_op_to_op_ack, updated service_registry_lifecycle
- `packages/server-rust/Cargo.toml` — Added `rmpv` dependency (required for wire value conversion in crdt.rs)

### Acceptance Criteria Status

- [x] AC1: LWW PUT — ClientOp with record merges into RecordStore, broadcasts ServerEvent(PUT), returns OpAck
- [x] AC2: LWW REMOVE — ClientOp with tombstone or op_type REMOVE, broadcasts ServerEvent(REMOVE), returns OpAck
- [x] AC3: OR_ADD — ClientOp with or_record, RecordStore::put with RecordValue::OrMap, broadcasts ServerEvent(OR_ADD)
- [x] AC4: OR_REMOVE — ClientOp with or_tag, RecordStore::put with RecordValue::OrTombstones, broadcasts ServerEvent(OR_REMOVE)
- [x] AC5: OpBatch processes all 3 ops, returns single OpAck with last_id from last op
- [x] AC6: Non-CRDT operations return Err(OperationError::WrongService)
- [x] AC7: CrdtService::new(...).name() returns "crdt"
- [x] AC8: full_pipeline_client_op_to_op_ack integration test passes (CrdtService wired in setup())
- [x] AC9: Empty OpBatch returns OperationResponse::Ack with ctx.call_id

### Deviations

- `rmpv` added to topgun-server Cargo.toml as direct dependency (the spec did not mention this, but rmpv is required in crdt.rs for type matching and is not re-exported from topgun-core)
- All G1/G2/G3/G4 tasks implemented in a single commit for crdt.rs; subsequent commits handled mod.rs and lib.rs changes (commit strategy matches spec intent)

---

## Review History

### Review v1 (2026-02-24)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1 (LWW PUT) — `apply_single_op` correctly calls `record_store.put()` with `RecordValue::Lww` and `CallerProvenance::CrdtMerge`, broadcasts `ServerEvent(PUT)`, returns `OpAck`
- [✓] AC2 (LWW REMOVE via tombstone) — `record: Some(None)` detected by `matches!(&op.record, Some(None))`, calls `record_store.remove()`, broadcasts `ServerEvent(REMOVE)`
- [✓] AC2 (LWW REMOVE via op_type) — `op.op_type.as_deref() == Some("REMOVE")` correctly handles the op_type string check
- [✓] AC3 (OR_ADD) — `or_record: Some(Some(_))` pattern matched, calls `record_store.put()` with `RecordValue::OrMap`, broadcasts `ServerEvent(OR_ADD)` with `or_record` and `or_tag` fields
- [✓] AC4 (OR_REMOVE) — `or_tag: Some(Some(_))` pattern matched when `or_record` absent, calls `record_store.put()` with `RecordValue::OrTombstones { tags: vec![tag.clone()] }`, broadcasts `ServerEvent(OR_REMOVE)`
- [✓] AC5 (OpBatch) — iterates all ops, `last_id` tracks last op's id, returns single `OpAck` with `last_id: "op-3"`; unit test at line 577 verifies `ack.payload.last_id == "op-3"` explicitly
- [✓] AC6 (WrongService) — `_ => Err(OperationError::WrongService)` catch-all in tower::Service dispatch; unit test at line 640 verifies `GarbageCollect` returns `Err(OperationError::WrongService)`
- [✓] AC7 (ManagedService name) — `name()` returns `service_names::CRDT` which is `"crdt"`; unit test at line 411 verifies
- [✓] AC8 (Integration test) — `full_pipeline_client_op_to_op_ack` in lib.rs (line 136) renamed correctly, asserts `OperationResponse::Message(msg) if matches!(**msg, Message::OpAck(_))`
- [✓] AC9 (Empty OpBatch) — early return `OperationResponse::Ack { call_id: ctx.call_id }` when `ops.is_empty()`; unit test at line 657 sets `ctx.call_id = 42` and asserts `Ack { call_id: 42 }`
- [✓] domain_stub!(CrdtService) removed — grep confirms no match in codebase
- [✓] crdt_service_returns_not_implemented test removed — grep confirms no match
- [✓] all_stubs_implement_managed_service updated — CrdtService removed from registry call; comment explains why
- [✓] lib.rs setup() wires real CrdtService::new() — uses `RecordStoreFactory` with `NullDataStore` and `connection_registry`
- [✓] service_registry_lifecycle updated — uses `CrdtService::new(record_store_factory, connection_registry_for_crdt)` with separate `ConnectionRegistry` instance
- [✓] rmpv_to_value handles all rmpv::Value variants including F32, F64, Nil, Ext
- [✓] Integer fallback: tries `as_i64()` first, falls back to `as_u64().unwrap_or(0) as i64` with `#[allow(clippy::cast_possible_wrap)]`
- [✓] Map key coercion: `k.to_string()` used for all key types as specified
- [✓] CoordinationService pattern followed exactly — struct layout, Arc wrapping, `#[must_use]` constructor, ManagedService no-ops
- [✓] No spec/phase references in code comments — WHY-comments used throughout
- [✓] No constraints violated — no partition routing, no conflict resolvers, no Merkle updates, no search index, etc.
- [✓] `cargo check` passes with zero errors
- [✓] `cargo clippy` passes with zero warnings

**Minor:**
1. AC5 test uses `record: None` on all three ops in the OpBatch test — these fall through to the LWW PUT branch with `lww_rec = None`, which skips the `store.put()` call. The spec AC5 says "RecordStore calls made for each" but the test only verifies `last_id` in the response, not storage interaction. This is a test coverage gap: no unit test verifies that `store.put()` is actually called with data for batched operations. Not a functional defect — the logic is correct, but a test with `record: Some(Some(lww_record))` for each batch op would provide stronger AC5 coverage.

**Summary:** The implementation is complete, correct, and follows established patterns precisely. All 9 acceptance criteria are implemented and tested. The code is clean, clippy-compliant, and integrates naturally into the existing service routing framework. The single minor finding (AC5 test uses no-op PUTs) is a test coverage improvement that does not affect correctness.

---

## Completion

**Completed:** 2026-02-24
**Total Commits:** 3
**Audit Cycles:** 2
**Review Cycles:** 1
