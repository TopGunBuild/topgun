# SPEC-063: Implement SyncService (Merkle Delta Sync Protocol)

```yaml
id: SPEC-063
type: feature
status: running
priority: P0
complexity: large
created: 2026-02-24
todo: TODO-086
```

## Context

The SyncService is the core offline-first protocol for TopGun. When a client reconnects after an offline period, it uses Merkle tree comparison to efficiently identify which records have changed, avoiding a full dataset transfer. This is the protocol that makes TopGun's offline-first architecture practical at scale.

Currently, `SyncService` is a `domain_stub!` macro that returns `OperationResponse::NotImplemented` for all operations. This spec replaces that stub with a real implementation that handles 6 sync-related Operation variants across two CRDT types (LWW-Map and OR-Map), introduces a `MerkleSyncManager` for per-partition Merkle tree management, and wires a `MerkleMutationObserver` into the storage layer so that Merkle trees stay in sync with RecordStore mutations automatically.

**TS behavioral references:**
- `packages/server/src/coordinator/lww-sync-handler.ts` (LwwSyncHandler)
- `packages/server/src/coordinator/ormap-sync-handler.ts` (ORMapSyncHandler)
- `packages/server/src/cluster/MerkleTreeManager.ts` (per-partition tree management)

**Existing Rust infrastructure:**
- `packages/core-rust/src/merkle.rs` -- `MerkleTree` (LWW) and `ORMapMerkleTree` (OR-Map) with update/remove/get_root_hash/get_buckets/get_keys_in_bucket/get_node/find_diff_keys/is_leaf APIs
- `packages/server-rust/src/storage/mutation_observer.rs` -- `MutationObserver` trait with on_put/on_update/on_remove/on_evict/on_load/on_clear/on_reset/on_destroy hooks
- `packages/server-rust/src/storage/factory.rs` -- `RecordStoreFactory` accepts `Vec<Arc<dyn MutationObserver>>` for observer injection

**Established pattern:** SPEC-061 (CoordinationService) and SPEC-062 (CrdtService) demonstrate how to replace a `domain_stub!` with a real `tower::Service<Operation>` implementation.

**Depends on:** SPEC-062 (CrdtService -- data must exist to sync), SPEC-058 (RecordStore/storage layer)

## Goal Analysis

**Goal Statement:** When a client reconnects after an offline period, it exchanges Merkle tree hashes with the server to identify exactly which records differ, then receives only the missing/changed records -- enabling efficient delta sync instead of full dataset transfer. RecordStore mutations automatically keep the server-side Merkle trees up to date.

**Observable Truths:**

1. A `SyncInit` message with a `map_name` returns a `SyncRespRoot` containing the server's current MerkleTree root hash for that map/partition, enabling the client to compare its local state against the server.
2. A `MerkleReqBucket` message with a `map_name` and `path` returns either `SyncRespBuckets` (intermediate node bucket hashes for further drilldown) or `SyncRespLeaf` (actual LWW records at leaf nodes for client consumption).
3. An `ORMapSyncInit` message returns an `ORMapSyncRespRoot` containing the server's ORMapMerkleTree root hash for that map/partition.
4. An `ORMapMerkleReqBucket` message returns either `ORMapSyncRespBuckets` (intermediate buckets) or `ORMapSyncRespLeaf` (actual OR-Map entries at leaf nodes).
5. An `ORMapDiffRequest` message with specific keys returns an `ORMapDiffResponse` containing the server's OR-Map entries for those keys.
6. An `ORMapPushDiff` message merges incoming OR-Map entries into the RecordStore and broadcasts changes to other clients.
7. When CrdtService writes data to the RecordStore, the MerkleMutationObserver automatically updates the partition's MerkleTree/ORMapMerkleTree, keeping the sync state consistent without explicit calls from the CRDT handler.

**Required Artifacts:**

| Artifact | Purpose |
|----------|---------|
| `packages/server-rust/src/service/domain/sync.rs` | SyncService struct, 6 handler methods, tower::Service impl, unit tests |
| `packages/server-rust/src/storage/merkle_sync.rs` | MerkleSyncManager (per-partition tree management) + MerkleMutationObserver (MutationObserver impl) |
| `packages/server-rust/src/service/domain/mod.rs` | Remove SyncService stub, add `pub mod sync` and re-export |
| `packages/server-rust/src/storage/mod.rs` | Add `pub mod merkle_sync` and re-export |
| `packages/server-rust/src/lib.rs` | Wire SyncService with MerkleSyncManager + RecordStoreFactory + ConnectionRegistry in integration tests |
| `packages/server-rust/src/service/domain/crdt.rs` | Visibility change only: `fn rmpv_to_value` -> `pub(crate) fn rmpv_to_value` (one-word addition required by AC14) |

**Key Links:**

- `SyncService` -> `MerkleSyncManager` (read Merkle tree state for sync responses)
- `SyncService` -> `RecordStoreFactory` (read records for leaf responses and ORMap diff responses)
- `SyncService` -> `ConnectionRegistry` (broadcast ServerEvent for ORMapPushDiff changes)
- `MerkleMutationObserver` -> `MerkleSyncManager` (update trees on RecordStore mutations)
- `RecordStoreFactory` observers list -> `MerkleMutationObserver` (injected at factory construction)
- `MerkleSyncManager` -> `topgun_core::merkle::{MerkleTree, ORMapMerkleTree}` (underlying tree implementations)

**Note on ObserverFactory integration:** `RecordStoreFactory` integration with a per-store `ObserverFactory` trait (to wire `MerkleMutationObserver` automatically) is deferred to a follow-up spec. For this spec, tests construct `MerkleMutationObserver` directly and wire it manually to `RecordStoreFactory` via the existing `Vec<Arc<dyn MutationObserver>>` constructor argument. This keeps the file count within the 6-file budget (the 6th file, `crdt.rs`, is a trivial visibility-only change -- see Language Profile exception note in Constraints).

## Task

Replace the `domain_stub!(SyncService, ...)` macro in `packages/server-rust/src/service/domain/mod.rs` with a real `SyncService` struct in a new `sync.rs` module, and create a `MerkleSyncManager` + `MerkleMutationObserver` in a new `storage/merkle_sync.rs` module that:

1. **MerkleSyncManager:** Maintains per-partition `MerkleTree` (LWW) and `ORMapMerkleTree` (OR-Map) instances. Provides closure-based accessors `with_lww_tree` and `with_ormap_tree`. Trees are lazily created on first access.

2. **MerkleMutationObserver:** Implements the `MutationObserver` trait. On `on_put`/`on_update`/`on_load`, extracts the key and computes an item hash from the RecordValue, then updates the appropriate Merkle tree in MerkleSyncManager. On `on_remove`/`on_evict`, removes the key from BOTH LWW and OR-Map trees (safe, since removing a non-existent key is a no-op). On `on_clear`/`on_reset`, clears the relevant partition's tree.

3. **SyncService:** Takes `Arc<MerkleSyncManager>`, `Arc<RecordStoreFactory>`, and `Arc<ConnectionRegistry>` as constructor dependencies. Implements `ManagedService` and `tower::Service<Operation>` for 6 Operation variants.

## Requirements

### Files to Create

**1. `packages/server-rust/src/storage/merkle_sync.rs`**

**MerkleSyncManager struct:**

```rust
use std::sync::Arc;
use dashmap::DashMap;
use parking_lot::Mutex;
use topgun_core::merkle::{MerkleTree, ORMapMerkleTree};

/// Per-partition Merkle tree manager for delta sync.
///
/// Maintains separate MerkleTree (LWW) and ORMapMerkleTree (OR-Map) instances
/// per (map_name, partition_id) pair. Trees are lazily created on first access.
pub struct MerkleSyncManager {
    /// Key: (map_name, partition_id) -> LWW MerkleTree
    lww_trees: DashMap<(String, u32), Mutex<MerkleTree>>,
    /// Key: (map_name, partition_id) -> OR-Map MerkleTree
    ormap_trees: DashMap<(String, u32), Mutex<ORMapMerkleTree>>,
    /// Default tree depth (3 = 4096 leaf buckets)
    depth: usize,
}
```

Methods:
- `new(depth: usize) -> Self` -- creates with configurable depth
- `default() -> Self` -- creates with depth 3
- `with_lww_tree<R>(&self, map_name: &str, partition_id: u32, f: impl FnOnce(&mut MerkleTree) -> R) -> R` -- lazily creates the tree if absent, locks the per-tree Mutex, passes a mutable reference to `f`, returns `f`'s result. Individual tree locks do not block unrelated partitions.
- `with_ormap_tree<R>(&self, map_name: &str, partition_id: u32, f: impl FnOnce(&mut ORMapMerkleTree) -> R) -> R` -- same pattern for OR-Map trees
- `update_lww(&self, map_name: &str, partition_id: u32, key: &str, item_hash: u32)` -- calls `with_lww_tree` to update the LWW tree entry
- `remove_lww(&self, map_name: &str, partition_id: u32, key: &str)` -- calls `with_lww_tree` to remove from the LWW tree
- `update_ormap(&self, map_name: &str, partition_id: u32, key: &str, entry_hash: u32)` -- calls `with_ormap_tree` to update the OR-Map tree entry
- `remove_ormap(&self, map_name: &str, partition_id: u32, key: &str)` -- calls `with_ormap_tree` to remove from the OR-Map tree
- `clear_partition(&self, map_name: &str, partition_id: u32)` -- removes both LWW and OR-Map trees for a partition from the DashMaps
- `clear_all(&self)` -- clears both DashMaps entirely

**Locking strategy:** `MerkleTree` and `ORMapMerkleTree` methods (`update`, `remove`, `get_root_hash`, `get_buckets`, etc.) take `&mut self`. The `MerkleSyncManager` uses `DashMap<(String, u32), parking_lot::Mutex<MerkleTree>>` (and same for ormap_trees) so that individual tree mutations do not block unrelated partitions. All access -- both reads and writes -- goes through the `with_lww_tree` / `with_ormap_tree` closure API, which locks only the specific tree's `Mutex` and never holds the DashMap shard lock across tree operations.

**Important:** Callers must NOT hold the `with_lww_tree` or `with_ormap_tree` closure open across `.await` points. All data needed from the tree (keys, hashes, node info) must be extracted within the closure and returned as owned values. Async operations (e.g., RecordStore fetches) must happen OUTSIDE the closure, after the Mutex is released.

**MerkleMutationObserver struct:**

```rust
/// MutationObserver implementation that keeps MerkleSyncManager in sync
/// with RecordStore mutations.
pub struct MerkleMutationObserver {
    manager: Arc<MerkleSyncManager>,
    map_name: String,
    partition_id: u32,
}
```

Note: The `MutationObserver` trait methods do not receive `map_name` or `partition_id` -- those are per-RecordStore attributes. The observer must be constructed with the map_name and partition_id it belongs to. For this spec, tests construct `MerkleMutationObserver` directly and pass it in the `Vec<Arc<dyn MutationObserver>>` argument of `RecordStoreFactory::new()`. The `ObserverFactory` trait (for automatic per-store creation) is deferred to a follow-up spec.

`MerkleMutationObserver` implements `MutationObserver`:
- `on_put(key, record, old_value, is_backup)`: If `is_backup`, skip (backup partitions don't participate in client sync). Compute item hash from `record.value` (see hash computation below). Call `manager.update_lww()` or `manager.update_ormap()` depending on `RecordValue` variant.
- `on_update(key, record, old_value, new_value, is_backup)`: If `is_backup`, skip. Compute hash from the `new_value: &RecordValue` parameter (NOT from `record.value`, which may not yet reflect the update). Call `manager.update_lww()` or `manager.update_ormap()` depending on the `new_value` variant.
- `on_remove(key, record, is_backup)`: If `is_backup`, skip (backup keys were never added to the tree). Call BOTH `manager.remove_lww()` AND `manager.remove_ormap()`. Removing a non-existent key is a harmless no-op, so calling both is safe and simpler than inspecting `record.value` to determine the original tree type.
- `on_evict(key, record, is_backup)`: If `is_backup`, skip. Call BOTH `manager.remove_lww()` AND `manager.remove_ormap()` for the same reason as `on_remove`.
- `on_load(key, record, is_backup)`: Same as `on_put` -- loading from storage should update the tree.
- `on_replication_put(key, record, populate_index)`: Update tree (replication data should be in sync state).
- `on_clear()`: Call `manager.clear_partition(map_name, partition_id)`.
- `on_reset()`: Call `manager.clear_partition(map_name, partition_id)`.
- `on_destroy(is_shutdown)`: Call `manager.clear_partition(map_name, partition_id)`.

**Hash computation for MerkleTree entries:**
- For `RecordValue::Lww { value, timestamp }`: Use `topgun_core::hash::fnv1a_hash(&format!("{}:{}:{}:{}", key, timestamp.millis, timestamp.counter, timestamp.node_id))` -- this matches the TS MerkleTree.update pattern.
- For `RecordValue::OrMap { records }`: Compute a combined hash from all records' tags (sorted for determinism): sort tags, concatenate with `|`, hash the result. Use `fnv1a_hash(&format!("key:{}|{}", key, sorted_tags_joined))`.
- For `RecordValue::OrTombstones { tags }`: Remove from the OR-Map tree (tombstones represent deletions).

**`value_to_rmpv` conversion helper:**

`sync.rs` defines a `pub(crate) value_to_rmpv(v: &topgun_core::types::Value) -> rmpv::Value` helper function for converting internal values to wire format. It is `pub(crate)` so that `AC14` can test it alongside `rmpv_to_value` from `crdt.rs` (also `pub(crate)`). A TODO comment should note that this and `rmpv_to_value` from `crdt.rs` should be consolidated into a shared `service/domain/conversion.rs` module in a follow-up spec.

**2. `packages/server-rust/src/service/domain/sync.rs`**

**Key `use` imports required:**

```rust
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use topgun_core::hash::fnv1a_hash;
use topgun_core::merkle::{MerkleTree, ORMapMerkleTree};
use topgun_core::messages::{self, Message};

use crate::network::connection::{ConnectionKind, ConnectionRegistry};
use crate::service::operation::{
    service_names, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::merkle_sync::MerkleSyncManager;
use crate::storage::{RecordStoreFactory, RecordStore};
```

**Struct definition:**

```rust
pub struct SyncService {
    merkle_manager: Arc<MerkleSyncManager>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
}
```

**Constructor:** `SyncService::new(merkle_manager, record_store_factory, connection_registry)`

**ManagedService impl:** Same pattern as CrdtService -- `name()` returns `service_names::SYNC`, `init`/`reset`/`shutdown` are no-ops.

**tower::Service<Operation> for Arc<SyncService>:**

Match on operation variants:
- `Operation::SyncInit { ctx, payload }` -> `handle_sync_init(&ctx, payload)`
- `Operation::MerkleReqBucket { ctx, payload }` -> `handle_merkle_req_bucket(&ctx, payload)`
- `Operation::ORMapSyncInit { ctx, payload }` -> `handle_ormap_sync_init(&ctx, payload)`
- `Operation::ORMapMerkleReqBucket { ctx, payload }` -> `handle_ormap_merkle_req_bucket(&ctx, payload)`
- `Operation::ORMapDiffRequest { ctx, payload }` -> `handle_ormap_diff_request(&ctx, payload)`
- `Operation::ORMapPushDiff { ctx, payload }` -> `handle_ormap_push_diff(&ctx, payload)`
- Any other variant -> `Err(OperationError::WrongService)`

**Note on incremental tower::Service implementation:** G3 implements the full `tower::Service<Operation>` dispatch match block, but only implements the LWW handler bodies (`handle_sync_init`, `handle_merkle_req_bucket`). The 4 OR-Map match arms (`ORMapSyncInit`, `ORMapMerkleReqBucket`, `ORMapDiffRequest`, `ORMapPushDiff`) must use `todo!("implemented in G4")` as placeholder bodies so the code compiles. G4 replaces these `todo!()` calls with the real implementations.

**Handler implementations:**

**`handle_sync_init`** (LWW sync initiation):
1. Extract `map_name` from `payload.map_name` — note: `SyncInitMessage` is a FLAT message; `map_name` is a direct field on the payload struct, NOT nested in a `.payload` sub-field (contrast with `MerkleReqBucketMessage` below, which has a `.payload` wrapper)
2. Get `partition_id` from `ctx.partition_id.unwrap_or(0)`
3. Get `root_hash` via `merkle_manager.with_lww_tree(map_name, partition_id, |tree| tree.get_root_hash())`
4. Build and return `OperationResponse::Message(Box::new(Message::SyncRespRoot(SyncRespRootMessage { payload: SyncRespRootPayload { map_name, root_hash, timestamp: ctx.timestamp.clone() } })))`

**`handle_merkle_req_bucket`** (LWW bucket/leaf drilldown):
1. Extract `map_name` and `path` from `payload.payload` -- note: `MerkleReqBucketMessage` is a WRAPPED message; `map_name` and `path` live in a nested `.payload` field (i.e., `payload.payload.map_name`), unlike the flat `SyncInitMessage`
2. Get `partition_id` from `ctx.partition_id.unwrap_or(0)`
3. Extract keys/node info from the tree within the closure scope, releasing the lock before any async operations:
   ```
   let node_data = merkle_manager.with_lww_tree(map_name, partition_id, |tree| {
       // extract all needed data (keys list OR bucket map) inside here, return as owned value
   });
   ```
4. If node had entries (leaf node): AFTER the closure returns and the Mutex is released, create a RecordStore via `record_store_factory.create(map_name, partition_id)`, fetch records for each key, build `SyncRespLeafPayload { map_name, path, records }` with `SyncLeafRecord` entries, return `Message::SyncRespLeaf`
5. If node is internal (no entries but has children): use the bucket hashes extracted in step 3, convert `HashMap<char, u32>` to `HashMap<String, u32>` (char -> single-char string), return `Message::SyncRespBuckets`
6. If node does not exist: return `OperationResponse::Empty` (no data at this path)

**`handle_ormap_sync_init`** (OR-Map sync initiation):
1. Extract `map_name` from `payload.map_name`
2. Get `partition_id` from `ctx.partition_id.unwrap_or(0)`
3. Get `root_hash` via `merkle_manager.with_ormap_tree(map_name, partition_id, |tree| tree.get_root_hash())`
4. Return `OperationResponse::Message(Box::new(Message::ORMapSyncRespRoot(ORMapSyncRespRoot { payload: ORMapSyncRespRootPayload { map_name, root_hash, timestamp: ctx.timestamp.clone() } })))`

**`handle_ormap_merkle_req_bucket`** (OR-Map bucket/leaf drilldown):
1. Extract `map_name` and `path` from `payload.payload`
2. Get `partition_id` from `ctx.partition_id.unwrap_or(0)`
3. Extract keys/node info within the closure scope, releasing the lock before any async operations:
   ```
   let node_data = merkle_manager.with_ormap_tree(map_name, partition_id, |tree| {
       // extract all needed data inside here, return as owned value
   });
   ```
4. If leaf: AFTER the closure returns and the Mutex is released, create RecordStore, fetch OR-Map entries for each key, build `ORMapSyncRespLeafPayload { map_name, path, entries }`, return `Message::ORMapSyncRespLeaf`
5. If internal: use bucket data extracted in step 3, convert keys, return `Message::ORMapSyncRespBuckets`
6. If node does not exist: return `OperationResponse::Empty`

**`handle_ormap_diff_request`** (OR-Map diff for specific keys):
1. Extract `map_name` and `keys` from `payload.payload`
2. Get `partition_id` from `ctx.partition_id.unwrap_or(0)`
3. Create RecordStore via `record_store_factory.create(map_name, partition_id)`
4. For each key, fetch the record from the store. If it is `RecordValue::OrMap`, convert to wire-format `ORMapEntry`. If absent, return empty records/tombstones for that key.
5. Return `Message::ORMapDiffResponse(ORMapDiffResponse { payload: ORMapDiffResponsePayload { map_name, entries } })`

**`handle_ormap_push_diff`** (OR-Map push changes from client):
1. Extract `map_name` and `entries` from `payload.payload`
2. Get `partition_id` from `ctx.partition_id.unwrap_or(0)`
3. Create RecordStore via `record_store_factory.create(map_name, partition_id)`
4. For each entry in `entries`: merge records into the RecordStore via `store.put(key, RecordValue::OrMap { records }, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)`
5. Broadcast `ServerEvent(OR_ADD)` for each entry/record to connected clients
6. Return `OperationResponse::Ack { call_id: ctx.call_id }`

**Record-to-wire conversion for leaf responses:**

For LWW leaf responses (`SyncRespLeaf`), records from the RecordStore (`RecordValue::Lww { value, timestamp }`) must be converted back to wire format `LWWRecord<rmpv::Value>`. This requires a `pub(crate) value_to_rmpv(v: &topgun_core::types::Value) -> rmpv::Value` helper defined in `sync.rs` (see note above about future consolidation). The helper recursively converts:
- `Value::Null` -> `rmpv::Value::Nil`
- `Value::Bool(b)` -> `rmpv::Value::Boolean(b)`
- `Value::Int(i)` -> `rmpv::Value::Integer(i.into())`
- `Value::Float(f)` -> `rmpv::Value::F64(f)`
- `Value::String(s)` -> `rmpv::Value::String(s.into())`
- `Value::Bytes(b)` -> `rmpv::Value::Binary(b.clone())`
- `Value::Array(a)` -> `rmpv::Value::Array(recursive)`
- `Value::Map(m)` -> `rmpv::Value::Map(recursive, keys as rmpv::Value::String)`

For OR-Map leaf/diff responses, records from the RecordStore (`RecordValue::OrMap { records: Vec<OrMapEntry> }`) must be converted to wire-format `Vec<ORMapRecord<rmpv::Value>>` using the same `value_to_rmpv` helper for each entry's value field.

### Files to Modify

**3. `packages/server-rust/src/service/domain/mod.rs`**

- Remove `domain_stub!(SyncService, service_names::SYNC);`
- Add `pub mod sync;`
- Add `pub use sync::SyncService;`
- Remove the `sync_service_returns_not_implemented` test (SyncService is no longer a stub)
- Update the `all_stubs_implement_managed_service` test: remove `registry.register(SyncService)` and remove `assert!(registry.get_by_name("sync").is_some())`

**4. `packages/server-rust/src/storage/mod.rs`**

- Add `pub mod merkle_sync;`
- Add `pub use merkle_sync::*;`

**5. `packages/server-rust/src/lib.rs`** (integration tests only)

- Update `setup()`: Create an `Arc<MerkleSyncManager>` with default depth. Replace `Arc::new(SyncService)` with `Arc::new(SyncService::new(merkle_manager.clone(), record_store_factory.clone(), connection_registry.clone()))`.
- Update any integration test that sends a sync Operation and expects `NotImplemented` to expect the correct sync response instead.
- Update `service_registry_lifecycle`: Replace `registry.register(SyncService)` with `registry.register(SyncService::new(merkle_manager, record_store_factory, connection_registry_for_sync))`.
- Construct `MerkleMutationObserver` directly and pass it in the `Vec<Arc<dyn MutationObserver>>` argument when creating `RecordStoreFactory` for tests that exercise the observer (AC8, AC9, AC10).

**6. `packages/server-rust/src/service/domain/crdt.rs`** (visibility change only)

- Change `fn rmpv_to_value(v: &rmpv::Value) -> Value` to `pub(crate) fn rmpv_to_value(v: &rmpv::Value) -> Value` (one-word addition).
- This is required by AC14 for the round-trip test.

## Acceptance Criteria

### AC1: SyncInit returns SyncRespRoot with correct root hash

Given a partition with LWW data (Merkle tree has non-zero root hash), when `Operation::SyncInit` is dispatched with `map_name: "users"`:
- Response is `OperationResponse::Message(Message::SyncRespRoot(...))` with `root_hash` matching the tree's current root hash
- `payload.timestamp` is present

### AC2: MerkleReqBucket returns SyncRespBuckets for internal nodes

Given a LWW Merkle tree with data at path "", when `Operation::MerkleReqBucket` is dispatched with `path: ""`:
- Response is `OperationResponse::Message(Message::SyncRespBuckets(...))` with a non-empty `buckets` map
- Each bucket key is a single hex character string ("0"-"f")

### AC3: MerkleReqBucket returns SyncRespLeaf for leaf nodes

Given a LWW Merkle tree with data and a path that reaches a leaf node, when `Operation::MerkleReqBucket` is dispatched with that path:
- Response is `OperationResponse::Message(Message::SyncRespLeaf(...))` with the correct `map_name` and `path` echoed in the payload
- `records` is non-empty when the RecordStore contains data at those keys; in unit tests using `NullDataStore` (which always returns `None`), `records` may be empty -- the test verifies the response message type and that `map_name`/`path` are correct
- Each record has `key` and `record` fields with valid LWWRecord data (verified in integration tests with a real MapDataStore)

### AC4: ORMapSyncInit returns ORMapSyncRespRoot

Given a partition with OR-Map data, when `Operation::ORMapSyncInit` is dispatched:
- Response is `OperationResponse::Message(Message::ORMapSyncRespRoot(...))` with `root_hash` matching the OR-Map tree's root hash

### AC5: ORMapMerkleReqBucket returns leaf or bucket response

Given OR-Map data, when `Operation::ORMapMerkleReqBucket` is dispatched:
- If path is a leaf: returns `Message::ORMapSyncRespLeaf` with the correct `map_name` and `path` echoed in the payload; `entries` is non-empty when the RecordStore contains data at those keys; in unit tests using `NullDataStore`, `entries` may be empty -- the test verifies message type and structure only
- If path is internal: returns `Message::ORMapSyncRespBuckets` with bucket hashes

### AC6: ORMapDiffRequest returns entries for requested keys

Given `Operation::ORMapDiffRequest` dispatched with `keys: ["key-1"]`:
- Response is `Message::ORMapDiffResponse` with the correct `map_name` in the payload
- When the RecordStore contains `RecordValue::OrMap` for "key-1", the response `entries` contains that key's data
- When the RecordStore does not contain the key (e.g., using `NullDataStore` in unit tests), the response returns empty records/tombstones for that key -- the test verifies the response message type and that missing keys are handled gracefully (not panicked or errored)

### AC7: ORMapPushDiff merges entries and broadcasts

Given `Operation::ORMapPushDiff` with entries:
- Each entry is merged into RecordStore via `put()` with `CallerProvenance::CrdtMerge`
- `ServerEvent(OR_ADD)` is broadcast to connected clients
- Response is `OperationResponse::Ack`

### AC8: MerkleMutationObserver updates tree on put/remove

Given a `MerkleMutationObserver` wired to a `MerkleSyncManager`:
- When `on_put` is called with a `RecordValue::Lww` record, the LWW tree's root hash changes to non-zero
- When `on_remove` is called for the same key, the LWW tree's root hash returns to zero
- Backup mutations (`is_backup: true`) do NOT update the tree

### AC9: MerkleMutationObserver updates OR-Map tree on put/remove

Given a `MerkleMutationObserver` wired to a `MerkleSyncManager`:
- When `on_put` is called with a `RecordValue::OrMap` record, the OR-Map tree's root hash changes to non-zero
- When `on_remove` is called for the same key, the OR-Map tree's root hash returns to zero

### AC10: MerkleSyncManager clear_partition resets trees

Given a `MerkleSyncManager` with trees for `("users", 0)`:
- After `clear_partition("users", 0)`, both LWW and OR-Map trees return root_hash = 0

### AC11: Wrong service returns WrongService error

Given any non-Sync `Operation` variant (e.g., `Operation::Ping`, `Operation::GarbageCollect`):
- Returns `Err(OperationError::WrongService)`

### AC12: ManagedService name is "sync"

`SyncService::new(...).name()` returns `"sync"`.

### AC13: Integration test -- SyncService replaces stub in full pipeline

The existing integration test `setup()` in `lib.rs` creates a real `SyncService`. A `SyncInit` message routed through the pipeline returns a `SyncRespRoot` (not `NotImplemented`).

### AC14: value_to_rmpv conversion round-trips with rmpv_to_value

Given any `topgun_core::types::Value`, converting to `rmpv::Value` via `value_to_rmpv` and back via `rmpv_to_value` (from CrdtService) produces an equivalent value. Both `value_to_rmpv` (in `sync.rs`) and `rmpv_to_value` (in `crdt.rs`) are `pub(crate)` so the round-trip test can access both from the same `#[cfg(test)]` module in either file. A TODO comment in each marks them for future extraction to a shared `service/domain/conversion.rs` module.

## Constraints

- Do NOT implement permission/access control checks (TS has `securityManager.checkPermission` -- defer to auth layer)
- Do NOT implement GC age check / `SYNC_RESET_REQUIRED` (TS checks `lastSyncTimestamp` against `gcAgeMs` -- defer to a future spec)
- Do NOT implement metrics tracking (TS has `metricsService.incOp` -- defer to metrics layer)
- Do NOT implement lazy map loading (`getMapAsync` pattern) -- RecordStore is always available via factory
- Do NOT implement partition ownership checks (defer to cluster routing)
- Do NOT implement write-through to PostgreSQL for ORMapPushDiff -- RecordStore handles persistence
- Do NOT modify the existing `MerkleTree` or `ORMapMerkleTree` APIs in core-rust
- Do NOT implement HTTP sync transport (TS has `http-sync-handler.ts` -- separate protocol)
- Do NOT implement worker pool offloading for Merkle operations (TS uses MerkleWorker -- not needed in Rust where there's no single-threaded constraint)
- All data access goes through `RecordStore` -- do NOT create in-memory LWWMap/ORMap instances directly
- Follow the `CrdtService` pattern exactly for struct layout, Arc wrapping, and test helpers
- Maximum 6 files (2 created + 4 modified) -- `crdt.rs` visibility change counts as a modification; `factory.rs` ObserverFactory changes are deferred to a follow-up spec. **Language Profile exception justified:** The Rust Language Profile sets a 5-file limit to prevent borrow-checker cascade risk. The 6th file (`crdt.rs`) is a single-word visibility change (`pub(crate)` added to one function signature) with zero cascade risk -- no trait boundaries change, no public API changes, no downstream ripple. This is an acceptable minimal exception.
- Do NOT hold a `with_lww_tree` or `with_ormap_tree` closure open across `.await` points -- extract all data within the closure, then perform async operations outside

## Assumptions

- `topgun_core::hash::fnv1a_hash` is publicly accessible and takes `&str`, returning `u32`. This is confirmed by its usage in `merkle.rs` tests.
- `topgun_core::merkle::MerkleTree` and `ORMapMerkleTree` take `&mut self` for `update()` and `remove()`. Interior mutability via `Mutex` in `MerkleSyncManager` is required.
- The `RecordValue` variant (`Lww` vs `OrMap` vs `OrTombstones`) determines which Merkle tree type (LWW vs OR-Map) to update in the observer.
- For leaf responses, `RecordStore::get(key, false)` can fetch records. The `false` parameter means "do not update access statistics" (read for sync should not affect LRU eviction).
- `NullDataStore` is sufficient for unit tests that verify response message types and structure. Leaf/diff handler unit tests accept that `records`/`entries` will be empty when `NullDataStore` is used. Integration tests in `lib.rs` that use a real `MapDataStore` verify non-empty record population (AC3, AC5, AC6 populated path).
- The `MerkleMutationObserver` does not need `map_name`/`partition_id` from the Record itself -- these are set at construction time per-RecordStore, which is the correct architectural approach since each RecordStore is scoped to a single (map_name, partition_id).
- `ConnectionRegistry::broadcast()` is used for ORMapPushDiff broadcasts, consistent with CrdtService. Sender exclusion is NOT implemented (same simplification as CrdtService).
- `ObserverFactory` integration into `RecordStoreFactory` is deferred. Tests for this spec wire `MerkleMutationObserver` manually.
- `rmpv_to_value` in `crdt.rs` is `pub(crate)` (updated as part of this spec). This is required for AC14's round-trip test.
- `on_remove` and `on_evict` call BOTH `remove_lww` and `remove_ormap`. Since removing a non-existent key from a DashMap-backed tree is a no-op (the key simply is not found), calling both is safe and avoids the need to inspect `record.value` to determine the original tree type.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Define `MerkleSyncManager` struct (DashMap+Mutex fields), constructor signatures, and `with_lww_tree`/`with_ormap_tree` closure API signatures. Define `MerkleMutationObserver` struct (no impl yet). Define `SyncService` struct with constructor and `ManagedService` impl stub (no handler logic). | -- | ~20% |
| G2 | 2 | Implement `MerkleSyncManager` method bodies: `update_lww`/`remove_lww`/`update_ormap`/`remove_ormap`/`clear_partition`/`clear_all`. Implement `MerkleMutationObserver` `MutationObserver` trait: `on_put`/`on_update`/`on_remove`/`on_evict`/`on_load`/`on_clear`/`on_reset`/`on_destroy` with hash computation (including `is_backup` guards on `on_remove` and `on_evict`; both call BOTH `remove_lww` and `remove_ormap`). Unit tests for AC8, AC9, AC10 in `merkle_sync.rs`. | G1 | ~25% |
| G3 | 2 | Implement `pub(crate) value_to_rmpv` conversion helper. Implement LWW handlers: `handle_sync_init`, `handle_merkle_req_bucket`. Implement `tower::Service<Operation>` dispatch for SyncService with `todo!("implemented in G4")` placeholders for the 4 OR-Map match arms. Unit tests for AC1, AC2, AC3, AC14. | G1 | ~25% |
| G4 | 3 | Implement OR-Map handlers: `handle_ormap_sync_init`, `handle_ormap_merkle_req_bucket`, `handle_ormap_diff_request`, `handle_ormap_push_diff`. Replace `todo!()` placeholders in Service dispatch with real calls. Unit tests for AC4, AC5, AC6, AC7. | G1, G3 | ~20% |
| G5 | 4 | Update `domain/mod.rs` (remove stub, add module). Update `storage/mod.rs` (add merkle_sync module). Update `lib.rs` (wire SyncService in integration tests, manual MerkleMutationObserver wiring). Update `crdt.rs` (`fn rmpv_to_value` -> `pub(crate) fn rmpv_to_value`). Tests for AC11, AC12, AC13. | G2, G3, G4 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |
| 4 | G5 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-24)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (5 groups across 6 files, large complexity)

**Critical:**

1. **File count exceeds Rust language profile limit.** The spec touches 6 files (2 created + 4 modified). PROJECT.md Language Profile sets "Max files per spec: 5" for Rust. The spec itself acknowledges this in the Warning section and suggests splitting into SPEC-063a and SPEC-063b. This must be resolved before implementation -- either split the spec or reduce file count to 5.

2. **Contradictory struct definition vs. locking strategy.** The `MerkleSyncManager` code block defines fields as `RwLock<HashMap<(String, u32), MerkleTree>>`, but the "Locking strategy" paragraph recommends `DashMap<(String, u32), parking_lot::Mutex<MerkleTree>>` instead. These are fundamentally different data structures. The implementer gets two contradictory blueprints. Pick one and update the struct definition to match. The DashMap+Mutex approach is technically superior (no global lock contention).

3. **`get_or_create_lww_tree` return type is unimplementable with DashMap.** The method signature `-> impl Deref<Target = MerkleTree>` requires returning a guard that holds both the DashMap entry reference and the inner Mutex lock. With `DashMap<K, Mutex<MerkleTree>>`, you cannot return a `MutexGuard<MerkleTree>` because the `DashMap::Ref` (or `DashMap::entry()` guard) must remain alive for the duration. This creates a self-referential borrow that Rust does not permit. Resolution: Either (a) replace `get_or_create_*` with purpose-specific read methods on `MerkleSyncManager` (e.g., `get_lww_root_hash(map_name, partition_id) -> u32`, `get_lww_buckets(map_name, partition_id, path) -> HashMap<char, u32>`, `get_lww_node(map_name, partition_id, path) -> Option<MerkleNode>` returning cloned data), or (b) use a callback/closure pattern `with_lww_tree<R>(&self, map_name, partition_id, f: impl FnOnce(&MerkleTree) -> R) -> R` that locks internally and passes a reference to the caller's closure.

**Recommendations:**

4. **[Compliance] `value_to_rmpv` and `rmpv_to_value` should be shared utilities.** Both `CrdtService` (`rmpv_to_value`) and `SyncService` (`value_to_rmpv`) need these conversion helpers. Currently `rmpv_to_value` is a private function in `crdt.rs`. Creating `value_to_rmpv` as a private function in `sync.rs` duplicates the pattern. Consider extracting both to a shared module (e.g., `service/domain/conversion.rs` or `storage/value_conversion.rs`) so future services can reuse them. This is not blocking but reduces maintenance burden.

5. **[Strategic] `on_update` should hash from `new_value`, not `record.value`.** The spec says `on_update` is "same as `on_put` -- recompute hash from `new_value`" but `on_put` computes from `record.value`. The `on_update` description should explicitly state it uses `new_value` (the 4th parameter), not `record.value`, since at the time of the observer call the record may or may not have been updated in place yet. The current wording is correct in intent but could confuse an implementer who copies the `on_put` logic verbatim.

6. **Trait-first ordering advisory.** G1 includes full `MerkleSyncManager` method implementations (not just struct + trait definitions). For strict trait-first compliance, G1 should contain only struct definitions, trait definitions (`ObserverFactory`), and constructor signatures. The method bodies (update_lww, remove_lww, etc.) are implementation, not type definitions. This is borderline -- the methods are simple delegation to DashMap/Mutex -- but worth noting for consistency with the Language Profile guidance.

7. **`handle_merkle_req_bucket` holds Merkle tree lock across async RecordStore::get calls.** Step 5 says "If node has entries (leaf node): create a RecordStore... fetch records for each key in entries." If the tree lock (Mutex) is held while calling `store.get(key, false).await`, the Mutex guard would be held across an await point. `parking_lot::MutexGuard` is not `Send`, which means holding it across `.await` will cause a compile error in async code. Resolution: extract the key list and entry data from the tree within the lock scope, drop the lock, THEN fetch records from RecordStore. This is important for correctness and should be specified explicitly.

### Response v1 (2026-02-24)
**Applied:** All 7 items (3 critical + 4 recommendations)

**Changes:**
1. [✓] File count exceeds limit — Deferred `factory.rs` ObserverFactory changes. Removed file 6 (`factory.rs`) from Required Artifacts table, Files to Modify, and Implementation Tasks. Updated Constraints to state "Maximum 5 files". Added note in Goal Analysis, Task section, and Assumptions explaining that `MerkleMutationObserver` is wired manually in tests for this spec. Updated G5 to reflect manual wiring in `lib.rs` instead of factory changes.
2. [✓] Contradictory struct definition — Replaced the `RwLock<HashMap<...>>` struct code block entirely with the `DashMap<(String, u32), parking_lot::Mutex<MerkleTree>>` definition. The locking strategy paragraph now consistently describes only the DashMap+Mutex approach.
3. [✓] `get_or_create_*` return type unimplementable — Replaced `get_or_create_lww_tree` and `get_or_create_ormap_tree` with `with_lww_tree<R>` and `with_ormap_tree<R>` closure-based APIs throughout: struct methods list, all handler descriptions, and locking strategy paragraph. Updated G1 description to reflect new API signature style.
4. [✓] Extract `value_to_rmpv`/`rmpv_to_value` to shared module — Deferred extraction to follow-up (adding a 3rd file would exceed the 5-file budget or require another deferral). Added explicit statement that `sync.rs` defines its own private `value_to_rmpv` with a TODO comment noting future consolidation into `service/domain/conversion.rs`.
5. [✓] `on_update` hash source explicit — Replaced "Same as `on_put` -- recompute hash from `new_value`" with: "Compute hash from the `new_value: &RecordValue` parameter (NOT from `record.value`, which may not yet reflect the update). Call `manager.update_lww()` or `manager.update_ormap()` depending on the `new_value` variant."
6. [✓] Trait-first G1 advisory — Moved method body implementations (`update_lww`/`remove_lww`/`update_ormap`/`remove_ormap`/`clear_partition`/`clear_all` and `MutationObserver` impl) from G1 to G2. G1 now contains only: struct definitions, constructor signatures, `with_lww_tree`/`with_ormap_tree` closure API signatures, `MerkleMutationObserver` struct (no impl), and `ManagedService` impl stub for `SyncService`.
7. [✓] Mutex guard held across `.await` — Added explicit instruction in both `handle_merkle_req_bucket` and `handle_ormap_merkle_req_bucket` descriptions to extract keys/entries inside the closure, then perform RecordStore fetches OUTSIDE the closure after the Mutex is released. Added a code pattern sketch. Added to Constraints: "Do NOT hold a `with_lww_tree` or `with_ormap_tree` closure open across `.await` points". Added warning to MerkleSyncManager locking strategy section.

### Audit v2 (2026-02-24)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (5 groups, large complexity with 1.3x business logic multiplier)

**Critical:**

1. **RecordStoreFactory.create() returns a fresh empty store on each call -- leaf/diff handler tests are unachievable as specified.** The handlers for `handle_merkle_req_bucket` (step 4), `handle_ormap_merkle_req_bucket` (step 4), and `handle_ormap_diff_request` (step 3) all call `record_store_factory.create(map_name, partition_id)`, which returns a brand-new `DefaultRecordStore` with empty `HashMapStorage`. With `NullDataStore`, `RecordStore::get()` always returns `None` for a fresh store. This means leaf responses (AC3, AC5) and diff responses (AC6) will always contain empty record lists, contradicting the ACs that require "records containing SyncLeafRecord entries" and "entries for key-1".

   **Resolution:** The spec must specify one of:
   - **(a)** For leaf/diff handler unit tests, the test creates a RecordStore from the factory, pre-populates it with `put()`, then passes that SAME store reference to the handler (bypassing the factory for reads). This requires making the handler methods accept a `&dyn RecordStore` parameter (test-seam pattern) or adding a store-cache to SyncService.
   - **(b)** For leaf/diff tests, use a mock `RecordStoreFactory` that caches and returns the same store instance for the same `(map_name, partition_id)`. This is straightforward since `RecordStoreFactory` is a concrete struct, so the test would need to either make it a trait or use a wrapper.
   - **(c)** For leaf/diff tests in `sync.rs`, test the conversion logic (value_to_rmpv + message building) separately from the store lookup. The handler integration test accepts that records will be empty (since NullDataStore is used), and the test verifies the response message type and structure only. Adjust AC3/AC5/AC6 wording to clarify: "records is non-empty when the RecordStore contains data at those keys" rather than asserting non-empty in unit tests.

   Option (c) is the simplest and most consistent with the CrdtService pattern (which also creates ephemeral stores per request and never tests read-back). Recommend option (c): revise AC3/AC5 to test message structure (correct type, correct path/map_name), and test record population in a follow-up spec when `RecordStoreFactory` gains caching or when an integration test uses a real `MapDataStore`.

**Recommendations:**

2. **`on_remove`/`on_evict` should check `is_backup` for consistency.** The spec skips backup mutations in `on_put`, `on_update`, and `on_load` (via the `is_backup` guard), but `on_remove` and `on_evict` do not check `is_backup`. Since backup puts are skipped (the tree never has backup keys), a backup remove/evict would be a harmless no-op. However, for implementation clarity and consistency, `on_remove` and `on_evict` should also check `is_backup` and skip if true. This prevents confusion and guards against future changes.

3. **[Strategic] AC14 tests a cross-module round-trip that depends on private functions.** `value_to_rmpv` is private in `sync.rs` and `rmpv_to_value` is private in `crdt.rs`. Testing that they are inverses requires either: (a) making both pub(crate), (b) duplicating `rmpv_to_value` in the sync test, or (c) testing indirectly through integration. The spec should specify which approach to use. Recommend (a): make both `pub(crate)` with a comment noting future extraction to a shared module.

4. **G3 `Est. Context` of ~25% includes both LWW handlers and tower::Service dispatch.** The tower::Service impl dispatches all 6 variants but only the LWW handlers are implemented in G3 (OR-Map handlers are in G4). When G3 is built, the 4 OR-Map match arms need placeholder returns (e.g., `Err(OperationError::WrongService)` or `todo!()`) that G4 later replaces. The spec should note this explicitly to avoid confusion: G3's Service impl should use `todo!("implemented in G4")` for the 4 OR-Map variants, or G3 should defer the full Service impl to G4.

### Response v2 (2026-02-24)
**Applied:** All 4 items (1 critical + 3 recommendations)

**Changes:**
1. [✓] RecordStoreFactory.create() empty store — Applied option (c). Revised AC3, AC5, and AC6 to clearly separate the unit-test assertion (message type + structure, with empty records acceptable when using NullDataStore) from the integration-test assertion (non-empty record population). Updated Assumptions to state that NullDataStore is sufficient for unit tests verifying response message types/structure, and that integration tests in lib.rs verify non-empty population. The ACs no longer assert "records is non-empty" unconditionally.
2. [✓] `on_remove`/`on_evict` is_backup guard — Added `is_backup` guard to both `on_remove` and `on_evict` in the MerkleMutationObserver spec. The description now reads "If `is_backup`, skip (backup keys were never added to the tree)" for both hooks. Updated G2 task description to note the `is_backup` guards on `on_remove` and `on_evict` explicitly.
3. [✓] AC14 cross-module private function access — Changed `value_to_rmpv` in sync.rs from `private` to `pub(crate)`. Specified that `rmpv_to_value` in `crdt.rs` must also be `pub(crate)` (updated in G5 if not already). Added to AC14: "Both `value_to_rmpv` and `rmpv_to_value` are `pub(crate)`". Added to Assumptions: "`rmpv_to_value` in crdt.rs is `pub(crate)` (updated as part of this spec if not already)." Updated `value_to_rmpv` description in Requirements to consistently use `pub(crate)`.
4. [✓] G3 tower::Service placeholder arms — Added explicit note in both the tower::Service dispatch section and the G3 task description: G3's Service impl uses `todo!("implemented in G4")` for the 4 OR-Map match arms. G4 replaces these `todo!()` calls with real implementations. Updated G5 to note that `rmpv_to_value` in `crdt.rs` should be confirmed/updated to `pub(crate)`.

### Audit v3 (2026-02-25)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~100% total (5 groups, 6 files, large complexity with 1.3x business logic multiplier)

**Per-Group Breakdown:**

| Group | Wave | Est. Context | Status |
|-------|------|--------------|--------|
| G1 | 1 | ~20% | OK |
| G2 | 2 | ~25% | OK |
| G3 | 2 | ~25% | OK |
| G4 | 3 | ~20% | OK |
| G5 | 4 | ~10% | OK |

**Quality Projection:** POOR range (cumulative ~100% across all groups)

**Critical:**

1. **File count (6) exceeds Rust language profile limit (5).** The spec states "Maximum 5 files" in Constraints but actually requires 6 files: 2 created (`sync.rs`, `merkle_sync.rs`) + 4 modified (`domain/mod.rs`, `storage/mod.rs`, `lib.rs`, `crdt.rs`). The `crdt.rs` modification (changing `fn rmpv_to_value` to `pub(crate) fn rmpv_to_value`) was added in Response v2 to satisfy AC14, but was never counted in the file list or reflected in the Constraints. The spec's "Files to Modify" section lists only 3 files (items 3-5), omitting `crdt.rs`.

   **Resolution options:**
   - **(a)** Add `crdt.rs` as file 6 in "Files to Modify" and update the Constraint from "Maximum 5 files" to "Maximum 6 files (2 created + 4 modified)." The `crdt.rs` change is a single-word visibility modifier addition (`pub(crate)`), making this a minimal exception to the 5-file rule. This is the simplest fix.
   - **(b)** Drop AC14 entirely and keep `value_to_rmpv` as a standalone `pub(crate)` function in `sync.rs` without testing the round-trip with `rmpv_to_value`. The round-trip test could be deferred to the follow-up spec that extracts both into `conversion.rs`. This keeps the file count at 5 but loses test coverage.
   - **(c)** Move the AC14 round-trip test into `sync.rs` and duplicate `rmpv_to_value` as a private test helper (copy the function body into the `#[cfg(test)]` module). This is ugly but avoids modifying `crdt.rs`. Not recommended.

   Recommend **(a)**: the `crdt.rs` change is trivially small (adding `pub(crate)` to one function signature) and the Language Profile limit exists to prevent borrow-checker cascade risk, which a visibility change does not introduce. Update the file list and constraint to reflect reality.

**Recommendations:**

2. **[Compliance] `on_remove`/`on_evict` dispatch: which tree to remove from?** The spec says `on_remove` and `on_evict` should "Call `manager.remove_lww()` or `manager.remove_ormap()`" depending on the `RecordValue` variant. However, `on_remove` receives `record: &Record`, which contains `record.value`. For `RecordValue::OrTombstones`, the spec says tombstones should be removed from the OR-Map tree. But `on_remove` does not know whether the original value was LWW or OR-Map -- a tombstone replaces the original. The implementer needs guidance: should `on_remove` call BOTH `remove_lww` and `remove_ormap` (safe, since removing a non-existent key is a no-op), or inspect `record.value` to determine the tree? Calling both is simpler and more robust. Consider specifying this explicitly.

3. **[Strategic] `SyncInitMessage` is a FLAT message but the handler accesses `payload.map_name`.** Looking at the actual Rust struct, `SyncInitMessage` has `map_name` directly on the struct (not nested in a `payload` field) -- it is a "FLAT message" per the doc comment. The handler description says "Extract `map_name` from `payload.map_name`" where `payload` refers to the Operation's payload field (`payload: messages::SyncInitMessage`). This is technically correct but could confuse an implementer who expects a `.payload.payload.map_name` double-unwrap (as with `MerkleReqBucketMessage` which does have a `.payload` field). The inconsistency is in the message schema design, not the spec, but a clarifying note would help.

### Response v3 (2026-02-25)
**Applied:** All 3 items (1 critical + 2 recommendations)

**Changes:**
1. [✓] File count (6) exceeds Language Profile limit (5) — Applied option (a). Added `crdt.rs` as a 6th row in the Required Artifacts table with purpose description. Updated the "Note on ObserverFactory integration" to reference the "6-file budget" instead of the outdated "5-file limit". Added explicit Language Profile exception justification to the Maximum 6 files constraint line: the 6th file is a single-word visibility change with zero cascade risk (no trait boundary changes, no public API changes, no downstream ripple), making it an acceptable minimal exception.
2. [✓] `on_remove`/`on_evict` dispatch ambiguity — Updated both `on_remove` and `on_evict` in the MerkleMutationObserver spec to call BOTH `remove_lww` AND `remove_ormap`, with explicit rationale: removing a non-existent key is a harmless no-op, so calling both is simpler and more robust than inspecting `record.value`. Updated the Task section description of MerkleMutationObserver to reflect this. Added a matching Assumption entry. Updated G2 task description to note both removes in `on_remove`/`on_evict`.
3. [✓] Flat vs. wrapped message structure note — Added clarifying note to `handle_sync_init` step 1: `SyncInitMessage` is a FLAT message so `map_name` is accessed directly as `payload.map_name` (not `payload.payload.map_name`). Added a contrasting note to `handle_merkle_req_bucket` step 1: `MerkleReqBucketMessage` is a WRAPPED message so `map_name` and `path` live in a nested `.payload` field. This gives implementers clear guidance on the inconsistency in the message schema design.

### Audit v4 (2026-02-25)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~100% total (5 groups, 6 files, 1.3x business logic multiplier)

**Scope:** Large (~100% estimated, exceeds 50% target). Implementation Tasks are already defined with proper wave decomposition. Each individual group is within the GOOD range (20-25%), so orchestrated parallel execution will maintain quality.

**Per-Group Breakdown:**

| Group | Wave | Est. Context | Status |
|-------|------|--------------|--------|
| G1 | 1 | ~20% | OK |
| G2 | 2 | ~25% | OK |
| G3 | 2 | ~25% | OK |
| G4 | 3 | ~20% | OK |
| G5 | 4 | ~10% | OK |

**Quality Projection:** Each worker operates in the GOOD range individually. Cumulative is high but groups are properly isolated.

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | Per-group target |
| 30-50% | GOOD | -- |
| 50-70% | DEGRADING | -- |
| 70%+ | POOR | Cumulative (mitigated by decomposition) |

**Audit Dimensions:**

- Clarity: PASS -- All handlers have step-by-step descriptions with explicit flat-vs-wrapped message notes
- Completeness: PASS -- 6 artifacts, 14 ACs, 12 constraints, 10 assumptions, all verified against source
- Testability: PASS -- Every AC has concrete assertions; unit vs integration distinction clearly stated
- Scope: PASS -- Boundaries explicitly constrained; deferred items clearly listed
- Feasibility: PASS -- DashMap+Mutex closure pattern verified implementable; all referenced APIs confirmed in source
- Architecture fit: PASS -- Follows established domain_stub! replacement pattern (SPEC-061, SPEC-062)
- Non-duplication: PASS -- value_to_rmpv duplication acknowledged with deferred extraction plan
- Cognitive load: PASS -- Clean separation of MerkleSyncManager/MerkleMutationObserver/SyncService
- Strategic fit: PASS -- SyncService is 3rd of 7 domain services on Phase 3 roadmap
- Project compliance: PASS -- MsgPack wire format, trait-first G1, CrdtService pattern followed

**Rust Auditor Checklist:**
- No `f64` for integer-semantic fields: PASS (u32 for hashes, u32 for partition_id)
- No `r#type: String` on message structs: N/A (no new message structs created)
- `Default` derived on payload structs: PASS (MerkleSyncManager has default())
- Enums for known value sets: N/A
- Wire compatibility via rmp_serde: PASS (uses existing core-rust message types)
- `#[serde(rename_all = "camelCase")]`: N/A (internal structs, not wire types)
- `#[serde(skip_serializing_if...)]`: N/A

**Language Profile:**
- File count: 6 (limit 5) -- exception justified (6th file is single-word visibility change)
- Trait-first: G1 defines only types/signatures, implementation in G2+ -- COMPLIANT

**Goal-Backward Validation:** All 7 observable truths have covering artifacts. All 6 key links are documented. No orphan artifacts.

**Comment:** This specification has matured significantly through 3 revision cycles. All previous critical issues (contradictory struct definitions, unimplementable return types, Mutex-across-await, file count inconsistencies, empty-store test impossibility, cross-module visibility) have been resolved. The spec is now clear, complete, and implementable. The Language Profile 6-file exception is minimal and well-justified. Ready for orchestrated parallel execution.

**Recommendation:** Use `/sf:run --parallel` -- groups are properly decomposed with correct wave ordering and dependency tracking.

## Execution Summary

**Executed:** 2026-02-25
**Mode:** orchestrated
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |
| 3 | G4 | complete |
| 4 | G5 | complete |

### Files Created

- `packages/server-rust/src/storage/merkle_sync.rs` — MerkleSyncManager, MerkleMutationObserver
- `packages/server-rust/src/service/domain/sync.rs` — SyncService with 6 handlers

### Files Modified

- `packages/server-rust/src/service/domain/mod.rs` — removed SyncService stub, added `pub mod sync`
- `packages/server-rust/src/service/domain/crdt.rs` — `pub(crate) fn rmpv_to_value`
- `packages/server-rust/src/storage/mod.rs` — added `pub mod merkle_sync`
- `packages/server-rust/src/lib.rs` — wired SyncService in integration tests

### Acceptance Criteria Status

- [x] AC1: SyncInit returns SyncRespRoot
- [x] AC2: MerkleReqBucket returns SyncRespBuckets for internal nodes
- [x] AC3: MerkleReqBucket returns SyncRespLeaf for leaf nodes
- [x] AC4: ORMapSyncInit returns ORMapSyncRespRoot
- [x] AC5: ORMapMerkleReqBucket returns ORMapSyncRespBuckets / ORMapSyncRespLeaf
- [x] AC6: ORMapDiffRequest returns ORMapDiffResponse
- [x] AC7: ORMapPushDiff stores entries and returns OpAck
- [x] AC8: MerkleMutationObserver updates LWW tree on put/remove
- [x] AC9: MerkleMutationObserver updates OR-Map tree on put/remove
- [x] AC10: clear_partition resets both trees
- [x] AC11: WrongService returns OperationError::WrongService
- [x] AC12: ManagedService name is "sync"
- [x] AC13: SyncService replaces stub — SyncInit through full pipeline returns SyncRespRoot
- [x] AC14: value_to_rmpv round-trips with rmpv_to_value

### Deviations

None.

### Commits

- `c6a90d9` — feat(sf-063): define MerkleSyncManager, MerkleMutationObserver, and SyncService skeletons
- `f70e323` — feat(sf-063): implement MerkleSyncManager, MerkleMutationObserver, LWW handlers, and value_to_rmpv
- `7b94ba0` — feat(sf-063): implement OR-Map handlers and complete sync service dispatch
- `78bc4d5` — feat(sf-063): wire SyncService in integration tests and fix clippy warnings
