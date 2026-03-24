---
id: SPEC-136d
type: feature
status: done
priority: P1
complexity: small
parent: SPEC-136
depends_on: [SPEC-136a, SPEC-136b]
created: 2026-03-21
source: TODO-070
---

# Shapes: Per-Shape Merkle Trees and Shape-Aware Sync

## Context

SPEC-136a established shape types, wire messages (including `ShapeSyncInitMessage`), and `Operation::ShapeSyncInit`. SPEC-136b implemented the `ShapeEvaluator` module and `ShapeRegistry`. This sub-spec adds per-shape Merkle trees for efficient delta sync on reconnect, and extends `ShapeService` to handle shape-specific Merkle sync init, with the traversal phase handled inside the existing `SyncService` bucket handler.

TopGun already has per-(map_name, partition_id) Merkle trees managed by `MerkleSyncManager` for full-map sync. This sub-spec creates a `ShapeMerkleSyncManager` that manages per-(shape_id, map_name, partition_id) Merkle trees, following the same patterns. When a client reconnects and re-subscribes to a shape, it sends a `ShapeSyncInitMessage` with its stored root hash. The server compares this against the per-shape Merkle tree and sends only the delta of matching records.

Shape Merkle sync reuses the existing `SyncRespRootMessage`, `SyncRespBucketsMessage`, and `SyncRespLeafMessage` wire protocol structs. Shape paths are distinguished by prefixing with the `shape_id` (e.g., `"<shape_id>/<partition_id>/<depth>/<bucket>"`).

## Task

Implement `ShapeMerkleSyncManager` for per-shape Merkle trees, add `handle_shape_sync_init` to `ShapeService` (where `ShapeSyncInit` is already routed), and extend `SyncService` bucket handler to detect shape-prefixed paths for the traversal phase.

## Requirements

### R1: ShapeMerkleSyncManager

**File:** `packages/server-rust/src/storage/shape_merkle.rs` (new)

`ShapeMerkleSyncManager` manages per-shape Merkle trees:

- Key: `(shape_id, map_name, partition_id)` -> `MerkleTree`
- Storage: `DashMap<(String, String, u32), Mutex<MerkleTree>>` — wrapping `MerkleTree` in `Mutex` to match the `MerkleSyncManager` pattern (avoids holding the DashMap shard lock during tree operations)

Methods:

- `fn new() -> Self`
- `fn init_tree(&self, shape_id: &str, map_name: &str, partition_id: u32, matching_keys: &[(String, u32)])` — build initial Merkle tree from matching record keys and their hashes. Hash type is `u32` to match `MerkleTree::update(key, item_hash: u32)`. Called by `ShapeService` on shape subscribe (SPEC-136c integration point).
- `fn update_entry(&self, shape_id: &str, map_name: &str, partition_id: u32, key: &str, hash: u32)` — update a single entry in the shape's Merkle tree when a matching mutation occurs. Locks the tree's `Mutex` directly without going through `with_tree`. Hash type is `u32` to match `MerkleTree::update`.
- `fn remove_entry(&self, shape_id: &str, map_name: &str, partition_id: u32, key: &str)` — remove an entry when a record no longer matches the shape (LEAVE event). Locks the tree's `Mutex` directly.
- `fn get_root_hash(&self, shape_id: &str, map_name: &str, partition_id: u32) -> u32` — returns `0` if tree does not exist. Return type matches `MerkleTree::get_root_hash() -> u32`.
- `fn aggregate_shape_root_hash(&self, shape_id: &str, map_name: &str) -> u32` — compute the aggregate root hash across all partitions for `(shape_id, map_name)` using `wrapping_add`. Iterates all DashMap entries where the key's first two elements match `shape_id` and `map_name`, accumulating `get_root_hash` values. Returns `0` if no partitions exist.
- `fn cleanup_shape(&self, shape_id: &str)` — remove all Merkle trees for a shape (on unsubscribe or disconnect). Uses `DashMap::retain` to remove all entries where the tuple's first element matches `shape_id`. `DashMap::retain` is safe for this purpose: it holds the shard lock internally while iterating, avoiding the deadlock risk of iteration-while-removing via external iterators.
- `fn with_tree<R>(&self, shape_id: &str, map_name: &str, partition_id: u32, f: impl FnOnce(&mut MerkleTree) -> R) -> Option<R>` — closure-based mutable access, locks the tree's Mutex and passes a **mutable** reference to `f`. Returns `None` if the tree does not exist. Matches `with_lww_tree` from `MerkleSyncManager` (which also takes `&mut MerkleTree`). Used by `SyncService` for Merkle traversal during shape-prefixed bucket requests.

Shape Merkle trees use the same depth (3) as regular Merkle trees.

**Unit tests:**

- Init tree, verify root hash is non-zero
- Update entry, verify root hash changes
- Remove entry, verify root hash changes
- Cleanup shape removes all trees for that shape_id
- Get root hash of non-existent tree returns 0
- Aggregate root hash sums across multiple partitions

### R2: Shape-aware Merkle sync init in ShapeService

**File:** `packages/server-rust/src/service/domain/shape.rs` (modify)

The classifier (`classify.rs` line 473) routes `Message::ShapeSyncInit` to `service_names::SHAPE`. The init handler is therefore added to `ShapeService` — which already holds `ShapeRegistry` — keeping routing and implementation aligned.

Add `handle_shape_sync_init` to `ShapeService`. The method uses the **client-driven traversal protocol**, matching the existing `SyncInit` / `MerkleReqBucket` flow:

**Protocol flow:**

1. Client sends `ShapeSyncInitMessage` (contains `shape_id: String` and `root_hash: u32` — no `map_name` field; `map_name` is resolved from the registry).
2. `handle_shape_sync_init` in `ShapeService`:
   a. Resolve `map_name` for the shape via `ShapeRegistry::get(shape_id)` to obtain the `ActiveShape`, which holds `shape.map_name`.
   b. Compute the server's aggregate shape root hash across all partitions for `(shape_id, map_name)` using `ShapeMerkleSyncManager::aggregate_shape_root_hash`.
   c. Respond with `SyncRespRootMessage { payload: SyncRespRootPayload { map_name, root_hash, timestamp } }`. `SyncRespRootPayload` has three fields: `map_name`, `root_hash`, and `timestamp` — there is no `path` field on this struct.
3. If the client's `root_hash` differs from the server's, the client drives subsequent traversal by sending `MerkleReqBucket` messages with shape-prefixed paths. **The existing `handle_merkle_req_bucket` handler in `SyncService` detects shape-prefixed paths** (paths beginning with a UUID-style `shape_id` segment rather than a 3-digit partition prefix) and routes to `ShapeMerkleSyncManager` instead of `MerkleSyncManager`. `handle_merkle_req_bucket` handles all traversal levels — returning `SyncRespLeafMessage` when the path reaches leaf depth.
4. Shape path format: `"<shape_id>/<partition_id>/<depth>/<bucket>"`. The shape prefix detection reuses the existing `parse_partition_prefix` function: if `parse_partition_prefix(path)` returns `None`, the path does not start with a 3-digit zero-padded partition prefix and is treated as a shape-prefixed path. The shape_id is the first `/`-delimited segment.
5. Leaf node responses for shape-prefixed paths contain only records that match the shape filter, with field projection applied via `ShapeEvaluator::apply_shape`. This reuses `SyncRespLeafMessage` with the same `records` field structure.

**ShapeService constructor change:**

Add `ShapeMerkleSyncManager` as an optional field to `ShapeService::new()`:

```rust
pub fn new(
    shape_registry: Arc<ShapeRegistry>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    shape_merkle_manager: Option<Arc<ShapeMerkleSyncManager>>,
) -> Self
```

`Option<Arc<ShapeMerkleSyncManager>>` keeps all unchanged `ShapeService` call sites working by passing `None`. Shape sync init is silently skipped (returns `OperationError::NotFound` or similar) when the field is `None`.

### R3: SyncService bucket traversal for shape-prefixed paths

**File:** `packages/server-rust/src/service/domain/sync.rs` (modify)

Extend `handle_merkle_req_bucket` in `SyncService` to detect shape-prefixed paths and route to `ShapeMerkleSyncManager`. `handle_merkle_req_bucket` handles all traversal levels — when the path reaches leaf depth it returns `SyncRespLeafMessage` directly, regardless of whether the path is shape-prefixed or regular. There is no separate leaf request operation or handler.

Add `ShapeMerkleSyncManager` and `ShapeRegistry` as optional fields to `SyncService` using a `new_basic` convenience constructor:

```rust
pub fn new(
    merkle_manager: Arc<MerkleSyncManager>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    shape_merkle_manager: Option<Arc<ShapeMerkleSyncManager>>,
    shape_registry: Option<Arc<ShapeRegistry>>,
) -> Self

/// Convenience constructor for callers that do not need shape sync.
/// Equivalent to `new(..., None, None)`.
pub fn new_basic(
    merkle_manager: Arc<MerkleSyncManager>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
) -> Self {
    Self::new(merkle_manager, record_store_factory, connection_registry, None, None)
}
```

All existing call sites inside `sync.rs` tests (`~10 helper calls`), `sim/cluster.rs` (line 128), and `benches/load_harness/main.rs` (line 510) call `SyncService::new_basic(...)` — requiring a name change from `new` to `new_basic` but no parameter changes. Only `lib.rs` calls the full `SyncService::new(...)` with `Some(...)` values. `test_server.rs` passes `shape_registry: None` in `AppState` and does not construct a `ShapeService`, so it also uses `SyncService::new_basic(...)`, not the full 5-parameter constructor.

### R4: Module registration

**File:** `packages/server-rust/src/storage/mod.rs` (modify)

Add:
```rust
pub mod shape_merkle;
```

### R5: SyncService and ShapeService wiring in lib.rs and test_server.rs

**File:** `packages/server-rust/src/lib.rs` (modify)
**File:** `packages/server-rust/src/bin/test_server.rs` (modify)

**`lib.rs` (full wiring):** In the main service assembly path where `ShapeRegistry` is already available (added by SPEC-136c):

- Pass `Some(Arc::clone(&shape_merkle_manager))` and `Some(Arc::clone(&shape_registry))` to `SyncService::new(...)`.
- Pass `Some(Arc::clone(&shape_merkle_manager))` to `ShapeService::new(...)`.

**`test_server.rs` (name-only rename):** The test server passes `shape_registry: None` in `AppState` and does not construct a full `ShapeService`. The `SyncService` call in `test_server.rs` (line 294) changes from `SyncService::new(...)` to `SyncService::new_basic(...)` — a name-only substitution with no parameter changes and no `Some(...)` values.

**File count: 6 production files** (`shape_merkle.rs` new, `shape.rs` modify, `sync.rs` modify, `storage/mod.rs` modify, `lib.rs` modify, `test_server.rs` modify). This exceeds the Language Profile limit of 5. However, `storage/mod.rs` is a 1-line addition and `test_server.rs` is a name-only rename (`new` -> `new_basic`), so the spirit of the limit — controlling borrow-checker cascade risk — is not violated. The deviation is acknowledged explicitly. Additionally, `sim/cluster.rs` and `load_harness/main.rs` require the same mechanical `new` -> `new_basic` name rename (no parameter changes).

### R6: Mutation integration (acknowledgement)

`update_entry` and `remove_entry` on `ShapeMerkleSyncManager` are defined in R1 for completeness, but **the component that calls them on live CRDT mutations is not wired in this spec**. The existing `MerkleSyncManager` is updated via `MerkleMutationObserver` (observer pattern in `storage/merkle_sync.rs`). A future spec (`SPEC-136e` or a follow-on) will add a `ShapeMerkleMutationObserver` that calls `update_entry`/`remove_entry` when CRDT events arrive for keys that match registered shapes. Until that spec, the shape Merkle trees are populated only at subscribe time via `init_tree` and are not updated on subsequent mutations.

## Acceptance Criteria

1. `ShapeMerkleSyncManager` correctly initializes, updates, and removes per-shape Merkle trees
2. `cleanup_shape` removes all trees for a given shape_id
3. `get_root_hash` returns 0 for non-existent trees
4. On reconnect, client sends `ShapeSyncInit` and receives `SyncRespRootMessage` with the server's aggregate shape root hash
5. Client drives subsequent traversal via `MerkleReqBucket`; shape-prefixed paths route to `ShapeMerkleSyncManager` via `handle_merkle_req_bucket` (which handles all traversal levels including leaves)
6. Existing full-map sync (`SYNC_INIT`) continues to work unchanged (no regression)
7. Shape sync responses use existing `SyncRespRoot/Buckets/Leaf` message structs with shape-prefixed paths
8. `cargo test --release -p topgun-server` passes (all existing + new tests)

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` — all tests pass
2. Verify existing `SYNC_INIT` flow is not modified
3. Verify shape-prefixed paths do not collide with existing partition paths (`parse_partition_prefix` returns `None` for shape paths; returns `Some` for 3-digit zero-padded partition paths). Note: shape_ids that are 3-digit numeric strings (e.g., `"042"`) would collide — see Constraints.
4. Verify `sim/cluster.rs` and `load_harness/main.rs` compile after renaming `SyncService::new` to `SyncService::new_basic` at those call sites
5. Verify `test_server.rs` uses `SyncService::new_basic(...)` (not the 5-parameter `new(...)`)

## Constraints

- Do NOT modify the existing full-map Merkle sync protocol (`SYNC_INIT` / `SYNC_RESP_ROOT` / etc.) — shapes are additive
- Shape Merkle trees use the same depth (3) as regular Merkle trees
- Do NOT add DataFusion dependency
- `ShapeSyncInit` routes to `service_names::SHAPE` (per `classify.rs`) — do NOT change `classify.rs`
- shape_ids must not be 3-digit numeric strings (e.g., `"042"`). Such values would cause `parse_partition_prefix` to misidentify a shape-prefixed path as a regular partition path. Enforce this in `ShapeRegistry::register` by rejecting any `shape_id` that matches `^[0-9]{3}$`, or document the constraint prominently in the API. In practice, shape_ids are client-generated UUIDs or prefixed strings (e.g., `"s-1"`), which are not affected.

## Assumptions

1. Shapes are ephemeral — Merkle trees are cleaned up on disconnect/unsubscribe.
2. Shape Merkle trees use the same hashing algorithm as regular Merkle trees.
3. Shape counts are small enough that per-shape Merkle trees do not cause significant memory pressure.

## Audit History

### Audit v1 (2026-03-22 12:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~20% total

**Critical:**

1. **Hash type mismatch (u64 vs u32):** R1 specifies `init_tree(..., matching_keys: &[(String, u64)])` and `update_entry(..., hash: u64)` with `u64` hash parameters, but the existing `MerkleTree::update(key, item_hash: u32)` uses `u32`, `MerkleTree::get_root_hash()` returns `u32`, `MerkleSyncManager` uses `u32` throughout, and `ShapeSyncInitPayload.root_hash` is `u32`. All hash parameters in `ShapeMerkleSyncManager` must be `u32` to match the codebase. This violates the Rust Type Mapping rule: hash values should use consistent types with the existing `MerkleTree` API.

2. **R2 is underspecified — missing bucket/leaf handler details:** The existing Merkle sync protocol uses three separate handlers: `handle_sync_init` (root comparison), `handle_merkle_req_bucket` (bucket traversal), and `handle_merkle_req_leaf` (leaf data). R2 step 5 says "perform Merkle tree traversal to find differing buckets/leaves" but does not specify whether (a) the client drives the traversal by sending subsequent `MerkleReqBucket`/`MerkleReqLeaf` messages with shape-prefixed paths and the existing handlers need to detect the prefix, or (b) a new `handle_shape_sync_init` method sends the full delta in a single response. Without this, the implementer cannot know how to wire the multi-step protocol. Specify the exact protocol flow.

3. **Missing dependency injection for SyncService:** R2 requires `SyncService` to access `ShapeMerkleSyncManager` and `ShapeRegistry`, but the spec does not specify how these are injected. The current `SyncService::new()` takes `(MerkleSyncManager, RecordStoreFactory, ConnectionRegistry)`. The spec must specify: (a) adding `ShapeMerkleSyncManager` and `ShapeRegistry` as constructor parameters, and (b) updating all call sites (`lib.rs`, `test_server.rs`, `sim/cluster.rs`, `load_harness/main.rs`). These call-site changes count toward the 5-file limit.

4. **File count likely exceeds 5:** The spec lists 3 files (shape_merkle.rs, sync.rs, mod.rs), but the SyncService constructor change requires updating `lib.rs`, `test_server.rs`, `sim/cluster.rs`, and `load_harness/main.rs`. Even counting only production code, `lib.rs` and `test_server.rs` are mandatory. That makes 5 files minimum (shape_merkle.rs, sync.rs, mod.rs, lib.rs, test_server.rs), with `sim/cluster.rs` and `load_harness/main.rs` also needing changes. Either scope down (e.g., use `Option<Arc<ShapeMerkleSyncManager>>` with `None` at unchanged call sites) or explicitly list all modified files and confirm the count.

**Recommendations:**

5. **DashMap key should use Mutex wrapping (pattern alignment):** The existing `MerkleSyncManager` uses `DashMap<(String, u32), Mutex<MerkleTree>>` — wrapping `MerkleTree` in `Mutex` to avoid holding the DashMap shard lock during tree operations. R1 specifies `DashMap<(String, String, u32), MerkleTree>` without the `Mutex` wrapper. Follow the established pattern for consistency and correctness under concurrent access.

6. **AC numbering is inconsistent:** AC items 1-3 have no prefix, then AC4 is labeled "AC7" and AC5 is labeled "AC8". Use sequential numbering (AC1-AC7) for clarity.

7. **`get_tree` returns `Option<MerkleTree>` — ownership concern:** `MerkleTree` is a complex trie structure. Returning an owned `MerkleTree` via clone from a DashMap is expensive. The existing pattern uses `with_lww_tree` closure API to avoid cloning. Consider a `with_tree` closure API instead: `fn with_tree<R>(&self, shape_id, map_name, partition_id, f: impl FnOnce(&MerkleTree) -> R) -> Option<R>`.

8. **Missing: Who calls `update_entry`/`remove_entry` on mutations?** The spec defines these methods but doesn't specify which component calls them when a CRDT mutation occurs that affects a shape. The existing `MerkleSyncManager` is updated via `MerkleMutationObserver` (observer pattern). Should `ShapeMerkleSyncManager` also be updated via the observer, or should `CrdtService`/`ShapeService` call it directly? This is an integration gap that may belong in a future spec but should be acknowledged.

### Response v1 (2026-03-22)
**Applied:** All critical issues and all recommendations

**Changes:**
1. [✓] Hash type mismatch (u64 vs u32) — Changed all hash parameters in R1 (`init_tree` matching_keys, `update_entry` hash, `get_root_hash` return type) from `u64` to `u32` to match `MerkleTree::update(key, item_hash: u32)` and `get_root_hash() -> u32`.
2. [✓] R2 underspecified — Replaced vague "perform traversal" with explicit client-driven protocol: `handle_shape_sync_init` returns `SyncRespRootMessage`, then client drives `MerkleReqBucket`/`MerkleReqLeaf` with shape-prefixed paths detected inside existing handlers. Path format and detection logic specified. Added new R2 section replacing the old one.
3. [✓] Missing dependency injection — Added R2 constructor change specifying `Option<Arc<ShapeMerkleSyncManager>>` and `Option<Arc<ShapeRegistry>>` as new parameters. Documented `None, None` pattern for unchanged call sites.
4. [✓] File count — Added explicit R4 section listing all 5 production files (`shape_merkle.rs`, `sync.rs`, `storage/mod.rs`, `lib.rs`, `test_server.rs`). Confirmed `sim/cluster.rs` and `load_harness/main.rs` require no changes (pass `None, None`). File count stays at 5.
5. [✓] DashMap Mutex wrapping — Changed R1 storage type from `DashMap<(String, String, u32), MerkleTree>` to `DashMap<(String, String, u32), Mutex<MerkleTree>>` with explanation matching the MerkleSyncManager pattern rationale.
6. [✓] AC numbering inconsistent — Renumbered acceptance criteria to sequential AC1-AC7, removing the "AC7"/"AC8" labels.
7. [✓] `get_tree` ownership concern — Replaced `get_tree` method with `with_tree<R>` closure API in R1, matching `with_lww_tree` pattern. Updated R2 to reference `with_tree` for traversal.
8. [✓] Missing mutation integration — Added R5 explicitly acknowledging that `update_entry`/`remove_entry` are not wired to live mutations in this spec. Documents the `MerkleMutationObserver` precedent and defers the observer wiring to a future spec.

### Audit v2 (2026-03-22 15:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~22% total

**Critical:**

1. **Operation routing mismatch — `ShapeSyncInit` routes to SHAPE service, not SYNC.** The classifier in `classify.rs` (line 473) routes `Message::ShapeSyncInit` to `service_names::SHAPE`. However, R2 adds `handle_shape_sync_init` to `SyncService`, which is registered under `service_names::SYNC`. The operation will never reach `SyncService` — it will be dispatched to `ShapeService`, which returns `OperationError::WrongService` for unrecognized operations. Two options: (a) Change `classify.rs` to route `ShapeSyncInit` to `service_names::SYNC` — but this adds a 6th file, violating the 5-file limit. (b) Keep the handler in `ShapeService` and have it delegate to `ShapeMerkleSyncManager`/`ShapeRegistry` directly — but this changes the spec's architectural approach. The spec must resolve this routing conflict and account for the file impact.

2. **R2 step 1 incorrectly states `ShapeSyncInitMessage` contains `map_name`.** The actual `ShapeSyncInitPayload` (defined in SPEC-136a at `core-rust/src/messages/shape.rs:94-99`) has only two fields: `shape_id: String` and `root_hash: u32`. There is no `map_name` field. Step 2a correctly says "resolve the map_name via ShapeRegistry", so the resolution is straightforward: remove `map_name` from the step 1 description.

3. **R2 step 2c references a non-existent `path` field on `SyncRespRootMessage`.** `SyncRespRootPayload` has three fields: `map_name`, `root_hash`, and `timestamp`. It does NOT have a `path` field. Only `SyncRespBucketsPayload` has `path`. The phrase "The `path` field is `""` (root level)" must be removed. The existing `handle_sync_init` also does not set any path — it returns only the root hash, map_name, and timestamp.

4. **`sim/cluster.rs` and `load_harness/main.rs` WILL require changes.** R4 claims "no changes needed at those sites" and Validation Checklist item 4 says "compile without modification." This is incorrect. Rust does not have default parameters. Changing `SyncService::new()` from 3 to 5 parameters means every call site must be updated — including `sim/cluster.rs` (line 128), `load_harness/main.rs` (line 510), and ~10 test helper calls inside `sync.rs`. These are trivial `, None, None` additions, but they ARE source modifications. The file count becomes 7 production files (shape_merkle.rs, sync.rs, storage/mod.rs, lib.rs, test_server.rs, sim/cluster.rs, load_harness/main.rs), exceeding the 5-file Language Profile limit. Resolution options: (a) Accept 7 files since sim/cluster.rs and load_harness are infra, not domain code. (b) Add a `SyncService::new_basic()` convenience constructor that wraps `new(..., None, None)` so existing call sites use it without changes. (c) Acknowledge the limit is exceeded and justify.

**Recommendations:**

5. **[Strategic] Consider adding the handler to `ShapeService` instead of `SyncService`.** Since `ShapeSyncInit` already routes to `service_names::SHAPE`, adding the Merkle sync handler to `ShapeService` (which already holds `ShapeRegistry`) avoids the routing mismatch entirely. `ShapeService` would need `ShapeMerkleSyncManager` injected (already an `Arc`), but this is simpler than re-routing or adding `ShapeRegistry` to `SyncService`. The existing `handle_merkle_req_bucket`/`handle_merkle_req_leaf` handlers in `SyncService` would still need shape-path detection for the traversal phase, but the init handler lives where the operation is routed. Alternatively, re-route `ShapeSyncInit` to SYNC if all shape sync logic belongs together in SyncService.

6. **`aggregate_shape_root_hash` method missing from R1.** R2 step 2b describes computing an aggregate root hash across all partitions using `wrapping_add`, but R1 does not define an `aggregate_shape_root_hash` method. Either add this method to the R1 API or specify that `SyncService` iterates partitions itself using `get_root_hash`.

7. **`cleanup_shape` iteration pattern needs clarification.** The `DashMap` key is `(String, String, u32)` but `cleanup_shape` takes only `shape_id`. This requires iterating all entries and removing those where the first tuple element matches. The `DashMap::retain` method is appropriate, but this should be explicit since naive iteration-while-removing can deadlock with DashMap if done incorrectly.

8. **`with_tree` closure takes `&MerkleTree` (shared ref) but `update_entry` needs `&mut MerkleTree`.** R1 defines `with_tree` as `f: impl FnOnce(&MerkleTree) -> R` (shared reference), but `MerkleTree::update` requires `&mut self`. The `update_entry` and `init_tree` methods need mutable access. Either: (a) change `with_tree` to pass `&mut MerkleTree`, or (b) have `update_entry`/`init_tree` lock the Mutex directly without going through `with_tree`. The existing `MerkleSyncManager::with_lww_tree` passes `&mut MerkleTree`, suggesting option (a).

### Response v2 (2026-03-22)
**Applied:** All critical issues and all recommendations

**Changes:**
1. [✓] Operation routing mismatch — Moved `handle_shape_sync_init` from `SyncService` to `ShapeService` (R2 now targets `shape.rs`). `classify.rs` is not modified. The traversal phase (bucket/leaf) stays in `SyncService` (R3) since `MerkleReqBucket`/`MerkleReqLeaf` already route to SYNC.
2. [✓] R2 step 1 incorrect map_name claim — Corrected: `ShapeSyncInitMessage` contains only `shape_id` and `root_hash`. `map_name` is resolved from `ShapeRegistry` in step 2a.
3. [✓] R2 step 2c non-existent path field — Removed all reference to a `path` field on `SyncRespRootMessage`. Response now correctly specifies only `map_name`, `root_hash`, `timestamp`.
4. [✓] sim/cluster.rs and load_harness call sites — Added `SyncService::new_basic()` convenience constructor in R3. Existing call sites rename from `new` to `new_basic` (3-param signature unchanged). File count stays manageable: `sim/cluster.rs` and `load_harness/main.rs` need only a name change, not a parameter change.
5. [✓] Handler in ShapeService — Applied. `handle_shape_sync_init` is now in `ShapeService` (R2), aligning with routing. `ShapeService` receives `Option<Arc<ShapeMerkleSyncManager>>` via constructor.
6. [✓] aggregate_shape_root_hash missing from R1 — Added `aggregate_shape_root_hash` method to R1 API with full specification (iterates DashMap, wrapping_add across matching partitions).
7. [✓] cleanup_shape iteration pattern — Added explicit note that `DashMap::retain` is used and why it is safe (holds shard lock internally, no external iteration).
8. [✓] with_tree closure mutability — Changed `with_tree` signature to pass `&mut MerkleTree`, matching `with_lww_tree`. Noted that `update_entry`/`init_tree` lock the Mutex directly rather than going through `with_tree`.

### Audit v3 (2026-03-22 19:45)
**Status:** APPROVED

**Context Estimate:** ~22% total

**Comment:** The spec is well-structured after two revision rounds. All previous critical issues have been resolved. The architecture is sound: `ShapeSyncInit` handler in `ShapeService` (aligned with routing), traversal in `SyncService` (aligned with `MerkleReqBucket`/`MerkleReqLeaf` routing), `new_basic` constructor pattern to minimize call-site churn. The `ShapeMerkleSyncManager` API follows established `MerkleSyncManager` patterns (Mutex-wrapped trees, closure access, `wrapping_add` aggregation). R6 correctly defers mutation observer wiring to a future spec.

**Recommendations:**

1. **R5 incorrectly assumes `test_server.rs` has `ShapeRegistry` available.** The current `test_server.rs` passes `shape_registry: None` in `AppState` and does not construct a `ShapeService`. The `SyncService::new(...)` call at line 294 in `test_server.rs` should use `SyncService::new_basic(...)` (name-only change), not the full 5-parameter `new(...)` with `Some(...)` values. Only `lib.rs` needs the full wiring. The implementer will see this and adapt, but the spec text should say "only `lib.rs` calls the full `SyncService::new(...)` with `Some(...)` values; `test_server.rs` uses `new_basic(...)`."

2. **[Strategic] Shape path collision risk with numeric shape_ids.** The `parse_partition_prefix` function matches paths starting with 3 ASCII digits followed by `/`. If a `shape_id` happens to be a 3-digit numeric string (e.g., `"042"`), the path `"042/000/2/a"` would be incorrectly parsed as a regular partition path (partition 42). Current shape_ids in tests use non-numeric prefixes (e.g., `"s-1"`), so this is not an immediate bug. Consider either: (a) documenting that shape_ids must not be 3-digit numeric strings, or (b) adding a validation check in `ShapeRegistry::register`. Low priority since shape_ids are typically client-generated UUIDs or prefixed strings.

3. **File count is 6, not 5.** The spec lists `lib.rs` and `test_server.rs` with a `+` sign as if they are one item, but they are separate files. Actual count: `shape_merkle.rs` (1), `shape.rs` (2), `sync.rs` (3), `storage/mod.rs` (4), `lib.rs` (5), `test_server.rs` (6). Plus `sim/cluster.rs` and `load_harness/main.rs` need mechanical `new` -> `new_basic` renames. The Language Profile limit is 5. However, `storage/mod.rs` is a 1-line addition and `test_server.rs` is a name-only rename (`new` -> `new_basic`), so the spirit of the limit (controlling borrow-checker cascade risk) is not violated. Acknowledge this deviation explicitly.

### Response v3 (2026-03-22)
**Applied:** All three recommendations from audit v3

**Changes:**
1. [✓] R5 test_server.rs assumption — Corrected R3 and R5. R3 now explicitly states that only `lib.rs` calls the full `SyncService::new(...)` with `Some(...)` values; `test_server.rs` uses `SyncService::new_basic(...)`. R5 split into two separate subsections: `lib.rs` (full wiring) and `test_server.rs` (name-only rename to `new_basic`, no `Some(...)` values).
2. [✓] Shape path collision risk — Added constraint in Constraints section: shape_ids must not be 3-digit numeric strings (e.g., `"042"`), with enforcement guidance (`ShapeRegistry::register` should reject `^[0-9]{3}$` patterns or document the constraint). Added a note to Validation Checklist item 3 referencing the constraint.
3. [✓] File count acknowledgement — Updated R5 file count from "5" to "6" with explicit acknowledgement of the Language Profile limit deviation and justification that the spirit of the limit is not violated (`storage/mod.rs` is 1-line, `test_server.rs` is a name-only rename).

### Audit v4 (2026-03-22 21:30)
**Status:** APPROVED

**Context Estimate:** ~22% total

**Comment:** The spec is thorough and implementable after three revision rounds. All previous critical issues have been resolved. Architecture is sound and well-aligned with existing codebase patterns. The `new_basic` convenience constructor elegantly minimizes call-site churn. File count deviation (6 vs 5 limit) is well-justified and acknowledged.

**Recommendations:**

1. **`MerkleReqLeaf` does not exist in the codebase.** Multiple places in the spec reference `handle_merkle_req_leaf` and `MerkleReqLeaf` messages (R2 step 3, R3 first paragraph, AC5) as if they are existing protocol elements. In reality, there is no `MerkleReqLeaf` operation or message anywhere in the codebase. The existing `handle_merkle_req_bucket` handler handles ALL traversal levels -- when the requested path corresponds to a leaf node in the Merkle tree, it returns a `SyncRespLeafMessage` directly (not via a separate leaf request). The core instruction to "modify `handle_merkle_req_bucket` to detect shape-prefixed paths" is correct; the references to a non-existent `handle_merkle_req_leaf` handler are spurious and should be removed to avoid confusing the implementer. Affected locations: R2 line 66 ("SyncInit / MerkleReqBucket / MerkleReqLeaf flow"), R2 step 3, R3 first paragraph, AC5.

2. **Rust Type Mapping table vs MerkleTree hash type.** PROJECT.md's type mapping table says "Hash value -> u64". The spec correctly uses `u32` to match the existing `MerkleTree` API (`update(key, item_hash: u32)`, `get_root_hash() -> u32`). This is the right decision -- consistency with the existing API takes precedence. No action needed; noted for completeness.

### Response v4 (2026-03-22)
**Applied:** Recommendation 1 only

**Changes:**
1. [✓] Phantom `MerkleReqLeaf` references removed — Removed all references to the non-existent `MerkleReqLeaf` operation and `handle_merkle_req_leaf` handler. Affected locations:
   - Context paragraph: removed "and `SyncRespLeafMessage`" claim about separate leaf handler (leaf responses are already covered by bucket handler description).
   - Task paragraph: changed "bucket/leaf handlers" to "bucket handler".
   - R2 section title line 66: changed "SyncInit / MerkleReqBucket / MerkleReqLeaf flow" to "SyncInit / MerkleReqBucket flow".
   - R2 step 3: removed `handle_merkle_req_leaf` from the handler list; clarified that `handle_merkle_req_bucket` handles all traversal levels including leaves.
   - R3 first paragraph: removed "and `handle_merkle_req_leaf`"; added explicit clarification that `handle_merkle_req_bucket` handles all levels and returns `SyncRespLeafMessage` at leaf depth directly.
   - AC5: removed `MerkleReqLeaf` from the criterion; updated to reference only `MerkleReqBucket` with a note that it handles all levels including leaves.

**Skipped:**
2. [✗] Recommendation 2 (Rust Type Mapping table vs MerkleTree hash type) — informational only, no action needed per revision scope.

### Audit v5 (2026-03-22 22:15)
**Status:** APPROVED

**Context Estimate:** ~22% total

**Comment:** Spec is clean and ready for implementation. All previous critical issues (hash type mismatch, routing mismatch, missing DI, phantom MerkleReqLeaf references, file count, path collision) have been resolved across four revision rounds. The architecture is well-aligned with existing codebase patterns: `DashMap<..., Mutex<MerkleTree>>`, closure-based access via `with_tree`, `new_basic` convenience constructor, `Option<Arc<T>>` for optional dependencies. All claims verified against codebase -- `ShapeSyncInitPayload` fields, `SyncRespRootPayload` fields, `SyncService::new` call sites, `parse_partition_prefix` location, and absence of `MerkleReqLeaf`. Strategic fit confirmed: partial replication / Shapes is a table-stakes feature per PROJECT.md. Project compliance verified: no new dependencies, no out-of-scope work, MsgPack wire format preserved. Language profile: 6 files (deviation acknowledged and justified).

---

## Execution Summary

**Executed:** 2026-03-22
**Commits:** 5

### Files Created
- `packages/server-rust/src/storage/shape_merkle.rs` — ShapeMerkleSyncManager with DashMap<(String, String, u32), Mutex<MerkleTree>>, 7 unit tests

### Files Modified
- `packages/server-rust/src/storage/mod.rs` — added `pub mod shape_merkle;`
- `packages/server-rust/src/service/domain/sync.rs` — added new_basic(), extended SyncService with optional shape fields, added handle_shape_prefixed_bucket for shape-prefixed Merkle traversal
- `packages/server-rust/src/service/domain/shape.rs` — added optional ShapeMerkleSyncManager field to ShapeService, added handle_shape_sync_init, routed Operation::ShapeSyncInit in Service dispatch
- `packages/server-rust/src/lib.rs` — created shape_merkle_manager, passed Some(Arc) to SyncService::new() and ShapeService::new() in integration test setup
- `packages/server-rust/src/bin/test_server.rs` — renamed SyncService::new to new_basic
- `packages/server-rust/src/sim/cluster.rs` — renamed SyncService::new to new_basic
- `packages/server-rust/benches/load_harness/main.rs` — renamed SyncService::new to new_basic; fixed pre-existing bugs

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] 1. ShapeMerkleSyncManager correctly initializes, updates, and removes per-shape Merkle trees
- [x] 2. cleanup_shape removes all trees for a given shape_id
- [x] 3. get_root_hash returns 0 for non-existent trees
- [x] 4. On reconnect, client sends ShapeSyncInit and receives SyncRespRootMessage with the server's aggregate shape root hash
- [x] 5. Client drives subsequent traversal via MerkleReqBucket; shape-prefixed paths route to ShapeMerkleSyncManager via handle_merkle_req_bucket (which handles all traversal levels including leaves)
- [x] 6. Existing full-map sync (SYNC_INIT) continues to work unchanged (no regression) — 593 tests pass, all pre-existing sync tests pass
- [x] 7. Shape sync responses use existing SyncRespRoot/Buckets/Leaf message structs with shape-prefixed paths
- [x] 8. cargo test --release -p topgun-server passes — 593 passed, 0 failed

### Deviations
1. [Rule 1 - Bug] Fixed pre-existing missing QueryService PredicateBackend arg in load_harness/main.rs (was already broken before this spec)
2. [Rule 1 - Bug] Fixed pre-existing missing AppState.shape_registry field in load_harness/main.rs (added by SPEC-136c, load_harness was not updated)

### Notes
- File count is 8 (6 production + sim/cluster.rs + load_harness) as acknowledged in spec (spirit of 5-file limit not violated)
- partition_ids() helper method added to ShapeMerkleSyncManager (not in spec) for SyncService traversal — this is a minor addition needed for completeness but currently not called; with_tree() is used instead
- load_harness pre-existing bugs were blocking compilation of benches and needed fixing per Rule 1 (blocking issues)

---

## Review History

### Review v1 (2026-03-22)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Major:**

1. **`init_tree` not called during `handle_shape_subscribe` — shape Merkle trees are never populated**
   - File: `packages/server-rust/src/service/domain/shape.rs:235`
   - Issue: R1 states "`init_tree` is called by `ShapeService` on shape subscribe (SPEC-136c integration point)." `handle_shape_subscribe` never calls `init_tree` on `shape_merkle_manager`. As a result, all shape Merkle trees remain empty. `aggregate_shape_root_hash` always returns 0. When a reconnecting client sends its stored non-zero hash, the server returns 0, triggering a full traversal that yields zero records. The comment `// merkle_root_hash is 0 until SPEC-136d implements per-shape Merkle trees` is also incorrect — this IS SPEC-136d. The subscribe handler should call `shape_merkle_manager.init_tree(shape_id, map_name, partition_id, &matching_keys_with_hashes)` for each partition after scanning records.
   - Fix: In `handle_shape_subscribe`, after collecting `matching_records` per partition store, call `shape_merkle_manager.init_tree(...)` with `(key, item_hash)` pairs derived from the matching records. Compute `item_hash` from each record's timestamp or value hash (matching whatever hashing the existing `MerkleSyncManager` uses for LWW records). Update `merkle_root_hash` in `ShapeRespPayload` to the aggregate hash from `aggregate_shape_root_hash`.

2. **Unused `partition_ids` method on `ShapeMerkleSyncManager` — dead `pub` code**
   - File: `packages/server-rust/src/storage/shape_merkle.rs:169`
   - Issue: `partition_ids` is a `pub` method added beyond spec scope that is never called anywhere in the codebase (confirmed by grep). The Execution Summary acknowledges it is "currently not called." Dead public API increases maintenance surface and misleads future readers.
   - Fix: Either remove the method, or make it `pub(crate)` and document that it is reserved for future use.

**Minor:**

3. **Shape_id 3-digit collision constraint not enforced or documented in `ShapeRegistry::register`**
   - File: `packages/server-rust/src/service/domain/shape.rs:350`
   - Issue: The spec's Constraints section requires: "Enforce this in `ShapeRegistry::register` by rejecting any `shape_id` that matches `^[0-9]{3}$`, or document the constraint prominently in the API." Neither enforcement nor documentation is present in the `register` doc comment.
   - Fix: Add a doc comment to `register` noting that `shape_id` must not be a 3-digit numeric string (e.g., `"042"`), as such IDs collide with partition path prefixes in `handle_merkle_req_bucket`.

4. **Redundant double-check in shape-prefixed path detection**
   - File: `packages/server-rust/src/service/domain/sync.rs:283`
   - Issue: `if path.contains('/') { if let Some(slash_pos) = path.find('/') { ... } }` — the inner `if let` always succeeds when the outer `if` is true. The outer check is unnecessary.
   - Fix: Simplify to `if let Some(slash_pos) = path.find('/') { ... }`.

**Passed:**

- [✓] `ShapeMerkleSyncManager` struct follows `MerkleSyncManager` pattern: `DashMap<(String, String, u32), Mutex<MerkleTree>>` — correctly implemented
- [✓] All 7 required unit tests for `ShapeMerkleSyncManager` are present and pass (init/update/remove/cleanup/nonexistent/aggregate/aggregate-empty)
- [✓] `with_tree` passes `&mut MerkleTree` as specified — matches `with_lww_tree` pattern
- [✓] `cleanup_shape` uses `DashMap::retain` — safe pattern documented correctly
- [✓] `aggregate_shape_root_hash` uses `wrapping_add` over matching partition entries — correct
- [✓] `ShapeService::new` signature matches spec: `Option<Arc<ShapeMerkleSyncManager>>` as 4th parameter
- [✓] `SyncService::new` 5-parameter signature and `new_basic` 3-parameter convenience constructor — correctly implemented
- [✓] All call sites updated: `test_server.rs`, `sim/cluster.rs`, `load_harness/main.rs` all use `SyncService::new_basic(...)` — verified
- [✓] `lib.rs` passes `Some(Arc::clone(&shape_merkle_manager))` to both `SyncService::new()` and `ShapeService::new()` — verified
- [✓] `storage/mod.rs` — `pub mod shape_merkle;` added — verified
- [✓] `handle_shape_sync_init` is in `ShapeService` (correct routing, `ShapeSyncInit` routes to SHAPE service) — correctly implemented
- [✓] Shape-prefixed bucket detection in `handle_merkle_req_bucket`: `parse_partition_prefix` returns `None` → path contains `/` → `handle_shape_prefixed_bucket` — correct logic
- [✓] Shape leaf responses apply `shape_evaluator::apply_shape` field projection — AC5 requirement met in code
- [✓] `handle_shape_sync_init` returns `SyncRespRootMessage` with correct fields (`map_name`, `root_hash`, `timestamp`) — no phantom `path` field — correct
- [✓] `ShapeSyncInitMessage` `map_name` not assumed — resolved from `ShapeRegistry::get` — correct
- [✓] Full-map sync (`SYNC_INIT`) unmodified — existing 593 tests pass, no regression
- [✓] Build passes: `cargo test --release -p topgun-server` — 593 passed, 0 failed
- [✓] Clippy passes: `cargo clippy --release -p topgun-server -- -D warnings` — clean
- [✓] Pre-existing load_harness compilation bugs fixed (valid per Rule 1 — blocking issue)

**Summary:** The core infrastructure (`ShapeMerkleSyncManager`, `new_basic`, wiring in `lib.rs`) is well-implemented and follows established patterns. One major issue blocks correctness: `init_tree` is never called during subscribe, so shape Merkle trees are always empty, making AC4 functionally broken despite the handler existing. Additionally, the spec's documented constraint on shape_id format is absent from the API. These should be fixed before marking done.

### Fix Response v1 (2026-03-22)
**Applied:** All issues (1-4)

**Fixes:**
1. [✓] `init_tree` not called during subscribe — Added per-partition `init_tree()` call in `handle_shape_subscribe` with `(key, hash)` pairs computed via `fnv1a_hash` matching `compute_lww_hash` pattern. Updated `merkle_root_hash` in `ShapeRespPayload` to use `aggregate_shape_root_hash` instead of hardcoded 0.
2. [✓] Dead `partition_ids` method — Removed from `ShapeMerkleSyncManager`.
3. [✓] Shape_id 3-digit collision constraint — Added `# Note` doc section to `ShapeRegistry::register` documenting the restriction.
4. [✓] Redundant `path.contains('/')` check — Simplified to `if let Some(slash_pos) = path.find('/') { ... }`.
   - Commit: c67e22c

### Review v2 (2026-03-22)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] Fix v1 issue 1: `init_tree` called in `handle_shape_subscribe` per partition, using `fnv1a_hash` matching `compute_lww_hash` format — verified correct
- [✓] Fix v1 issue 2: Dead `partition_ids` method removed from `ShapeMerkleSyncManager` — confirmed absent
- [✓] Fix v1 issue 3: `ShapeRegistry::register` doc comment notes 3-digit shape_id restriction — present at line 384
- [✓] Fix v1 issue 4: Redundant `path.contains('/')` outer check removed — `if let Some(slash_pos) = path.find('/')` used directly
- [✓] `ShapeMerkleSyncManager` correctly implements all R1 methods: `init_tree`, `update_entry`, `remove_entry`, `get_root_hash`, `aggregate_shape_root_hash`, `cleanup_shape`, `with_tree`
- [✓] All 7 unit tests present and passing (init/update/remove/cleanup/nonexistent/aggregate-sum/aggregate-empty)
- [✓] `DashMap<(String, String, u32), Mutex<MerkleTree>>` pattern matches `MerkleSyncManager`
- [✓] `cleanup_shape` uses `DashMap::retain` — safe pattern
- [✓] `with_tree` passes `&mut MerkleTree` matching `with_lww_tree` pattern
- [✓] `aggregate_shape_root_hash` uses `wrapping_add` — correct
- [✓] `merkle_root_hash` in `ShapeRespPayload` now uses `aggregate_shape_root_hash` (not hardcoded 0)
- [✓] Hash computation in subscribe matches `compute_lww_hash` format exactly: `"key:millis:counter:node_id"`
- [✓] `handle_shape_sync_init` in `ShapeService` — correct routing (SHAPE service handles ShapeSyncInit)
- [✓] `SyncRespRootMessage` fields correct: `map_name`, `root_hash`, `timestamp` — no phantom `path` field
- [✓] `map_name` resolved from `ShapeRegistry::get` in `handle_shape_sync_init` — not assumed from message
- [✓] `SyncService::new` 5-parameter and `new_basic` 3-parameter constructors — both implemented correctly
- [✓] All call sites: `test_server.rs`, `sim/cluster.rs`, `load_harness/main.rs` use `new_basic` — verified
- [✓] `lib.rs` integration test setup passes `Some(Arc::clone(&shape_merkle_manager))` to both `SyncService::new()` and `ShapeService::new()` — verified
- [✓] `storage/mod.rs` — `pub mod shape_merkle;` added — verified
- [✓] Shape-prefixed path detection uses `parse_partition_prefix` returns `None` → `path.find('/')` pattern — correct
- [✓] `handle_shape_prefixed_bucket` applies `shape_evaluator::apply_shape` field projection at leaf — AC5 met
- [✓] Shape Merkle sync uses existing `SyncRespRoot/Buckets/Leaf` message structs with shape-prefixed paths — AC7 met
- [✓] Full-map sync (`SYNC_INIT`) unmodified — 593 tests pass, no regression
- [✓] `cargo test --release -p topgun-server`: 593 passed, 0 failed
- [✓] `cargo clippy --release -p topgun-server -- -D warnings`: clean

**Summary:** All four issues from Review v1 were correctly applied. The implementation is complete, correct, and follows established codebase patterns throughout. The shape Merkle trees are now properly populated at subscribe time, the dead `partition_ids` method is removed, the 3-digit collision constraint is documented, and the redundant path check is simplified. Build and clippy are clean.

---

## Completion

**Completed:** 2026-03-22
**Total Commits:** 6
**Review Cycles:** 2

### Outcome

Delivered per-shape Merkle trees (`ShapeMerkleSyncManager`) and shape-aware sync protocol, enabling efficient delta sync on reconnect for partial replication shapes. Shape subscribe now populates Merkle trees, and shape-prefixed bucket traversal routes through the existing sync protocol.

### Key Files

- `packages/server-rust/src/storage/shape_merkle.rs` — ShapeMerkleSyncManager with DashMap-backed per-(shape_id, map_name, partition_id) Merkle trees
- `packages/server-rust/src/service/domain/shape.rs` — handle_shape_sync_init + init_tree wiring in subscribe
- `packages/server-rust/src/service/domain/sync.rs` — shape-prefixed bucket detection in handle_merkle_req_bucket, new_basic convenience constructor

### Patterns Established

- `SyncService::new_basic()` convenience constructor pattern for callers that don't need shape sync (avoids `, None, None` at every call site)
- Shape-prefixed path format `"<shape_id>/<partition_id>/<depth>/<bucket>"` detected via `parse_partition_prefix` returning `None`

### Deviations

- File count 8 (6 production + sim/cluster.rs + load_harness) vs 5-file Language Profile limit — justified by mechanical renames and 1-line additions
- Fixed 2 pre-existing load_harness compilation bugs (missing QueryService PredicateBackend arg and AppState.shape_registry field from SPEC-136c)
