---
id: SPEC-119
type: refactor
status: done
priority: high
complexity: medium
created: 2026-03-17
---

# SPEC-119: Eliminate Merkle Partition 0 Dual-Write via Scatter-Gather Root Hash

## Context

Every mutation in the system dual-writes to two Merkle trees: the key's actual partition tree AND a global aggregate at partition 0. This design was introduced in SPEC-080 to fix late-joiner sync (TODO-111), because the TS client sends `SYNC_INIT` without a partition hint and the server needs a single root hash covering all data.

The problem: all 271 partition workers compete for the same `Mutex<MerkleTree>` at `(map_name, 0)`. This negates the per-partition parallelism achieved by SPEC-116/117/118. Benchmarks show throughput collapses from 196 ops/sec (1 VU) to 126 ops/sec (200 VUs) with p50=5.7s latency.

Hazelcast uses strictly per-(IMap, partition) Merkle trees with no global aggregate. Anti-entropy fans out to all partitions independently.

**Research:** RES-002 confirmed Option 1 (Scatter-Gather Root Hash) as the recommended approach. The server synthesizes the root hash at sync time by iterating all partition trees, eliminating the hot-path dual-write entirely. No client protocol changes required.

## Task

Replace the partition 0 dual-write with a scatter-gather approach:

1. Remove all dual-write logic from `MerkleMutationObserver` (each observer writes only to its own partition)
2. Add `aggregate_root_hash()` and `aggregate_ormap_root_hash()` methods to `MerkleSyncManager` that iterate all partition trees for a map_name and combine root hashes via `wrapping_add`
3. Add `aggregate_buckets()` and `aggregate_ormap_buckets()` methods that combine bucket hashes across partitions for root-level bucket requests
4. Update `SyncService` handlers to use scatter-gather: `handle_sync_init` calls `aggregate_root_hash()`, `handle_merkle_req_bucket` encodes partition ID in path prefix for sub-root traversal
5. Use path encoding convention: root path `""` triggers aggregate mode; paths starting with 3-digit partition prefix (e.g., `"042/abc"`) route to specific partition tree

## Goal Analysis

**Goal Statement:** Eliminate the partition 0 Mutex bottleneck so per-partition write parallelism scales linearly under concurrent load without breaking client delta sync.

**Observable Truths:**
1. No Merkle tree exists at partition ID 0 (unless actual data hashes to partition 0) -- the `CLIENT_SYNC_PARTITION` constant and dual-write code are deleted
2. A client `SYNC_INIT` receives a root hash that reflects ALL records across all partitions
3. A client `MERKLE_REQ_BUCKET` at the root level receives bucket hashes aggregated from all partitions
4. A client `MERKLE_REQ_BUCKET` at a sub-root level is routed to the correct per-partition tree via path prefix
5. Leaf record fetches still resolve to the correct per-key partition via `hash_to_partition(key)` (unchanged)
6. Integration tests (TS client sync) continue to pass without client-side changes

**Required Artifacts:**
- `merkle_sync.rs` -- aggregate methods on `MerkleSyncManager`, simplified `MerkleMutationObserver`
- `sync.rs` -- updated handlers using scatter-gather instead of partition 0

**Key Links:**
- `MerkleMutationObserver::update_tree()` -> `MerkleSyncManager` (removes dual-write)
- `SyncService::handle_sync_init()` -> `MerkleSyncManager::aggregate_root_hash()` (new)
- `SyncService::handle_merkle_req_bucket()` -> path prefix decoding -> `MerkleSyncManager::with_lww_tree()` (routing)

## Requirements

### File: `packages/server-rust/src/storage/merkle_sync.rs`

**MerkleSyncManager -- new methods:**

```rust
/// Aggregates LWW root hashes across all partitions for `map_name`.
/// Returns `wrapping_add` of all per-partition root hashes. Empty = 0.
pub fn aggregate_lww_root_hash(&self, map_name: &str) -> u32

/// Aggregates OR-Map root hashes across all partitions for `map_name`.
pub fn aggregate_ormap_root_hash(&self, map_name: &str) -> u32

/// Aggregates LWW bucket hashes at `path` across all partitions for `map_name`.
/// For each hex bucket character, wrapping_add all partition values.
/// Returns HashMap<char, u32> with combined hashes.
pub fn aggregate_lww_buckets(&self, map_name: &str, path: &str) -> HashMap<char, u32>

/// Aggregates OR-Map bucket hashes at `path` across all partitions.
pub fn aggregate_ormap_buckets(&self, map_name: &str, path: &str) -> HashMap<char, u32>

/// Returns all partition IDs that have a LWW tree for `map_name`.
pub fn lww_partition_ids(&self, map_name: &str) -> Vec<u32>

/// Returns all partition IDs that have an OR-Map tree for `map_name`.
pub fn ormap_partition_ids(&self, map_name: &str) -> Vec<u32>
```

**MerkleMutationObserver -- simplify:**
- Delete `const CLIENT_SYNC_PARTITION: u32 = 0`
- In `update_tree()`: remove the `if self.partition_id != Self::CLIENT_SYNC_PARTITION` blocks; write only to `self.partition_id`
- In `on_remove()`: remove dual-write to partition 0
- In `on_evict()`: remove dual-write to partition 0

### File: `packages/server-rust/src/service/domain/sync.rs`

**handle_sync_init():**
- Replace `ctx.partition_id.unwrap_or(0)` with call to `merkle_manager.aggregate_lww_root_hash(&map_name)`
- Remove `with_lww_tree` call for root hash

**handle_merkle_req_bucket():**
- If path is `""` (root): call `merkle_manager.aggregate_lww_buckets(&map_name, "")` to get combined bucket hashes; in the response, prefix each bucket character with partition-aware paths. Specifically, return bucket hashes directly (aggregated) and for leaf requests at root, collect keys from all partitions.
- If path starts with 3-digit partition prefix (e.g., `"042/abc"`): parse partition ID, strip prefix, route to `merkle_manager.with_lww_tree(map_name, partition_id, ...)`
- If path has no prefix and is not empty: this is a root-level bucket (single hex char like `"a"`). Aggregate across all partitions: collect keys/buckets from each partition's sub-tree at that path, combine hashes for internal nodes. For leaf nodes, collect all keys from all partitions that have entries at this path, prefix the returned path with partition ID for subsequent drill-down.

**handle_ormap_sync_init():**
- Same pattern: replace `ctx.partition_id.unwrap_or(0)` with `merkle_manager.aggregate_ormap_root_hash(&map_name)`

**handle_ormap_merkle_req_bucket():**
- Same scatter-gather pattern as LWW variant

### Path Encoding Convention

The path traversal works in two modes:

1. **Aggregate mode** (paths without partition prefix -- `""`, `"a"`, `"a3"`): The server aggregates across all partitions. Each partition tree has the same depth structure, so bucket characters (`0`-`f`) are consistent. Hashes are combined via `wrapping_add`. When a node is a leaf in ANY partition, the server collects keys from all partitions at that path and returns them with partition-prefixed paths for subsequent requests.

2. **Routed mode** (paths with partition prefix -- `"042/a3c"`): The server strips the prefix, routes to the specific partition tree, and returns results directly. Subsequent child paths inherit the prefix.

### Deletions

- `MerkleMutationObserver::CLIENT_SYNC_PARTITION` constant
- All `if self.partition_id != Self::CLIENT_SYNC_PARTITION { ... }` blocks in `update_tree()`, `on_remove()`, `on_evict()`
- Tests: `dual_write_lww_put_updates_partition_0`, `dual_write_ormap_put_updates_partition_0`, `dual_write_remove_clears_from_partition_0`, `dual_write_evict_clears_from_partition_0`, `dual_write_replication_put_updates_partition_0`, `on_clear_does_not_clear_partition_0_for_non_zero_partition`, `partition_0_observer_does_not_double_write`

## Acceptance Criteria

1. **No dual-write:** `MerkleMutationObserver::update_tree()` writes to exactly one tree (`self.partition_id`). The `CLIENT_SYNC_PARTITION` constant does not exist.
2. **Aggregate root hash:** `MerkleSyncManager::aggregate_lww_root_hash("users")` returns the `wrapping_add` of all `(users, *)` partition root hashes. Returns 0 when no partitions exist.
3. **Aggregate root hash is commutative:** The result is identical regardless of DashMap iteration order (wrapping_add is commutative and associative).
4. **SyncService uses aggregate:** `handle_sync_init()` calls `aggregate_lww_root_hash()`, not `with_lww_tree(..., 0, ...)`.
5. **Path prefix routing:** `handle_merkle_req_bucket()` with path `"042/abc"` routes to partition 42, sub-path `"abc"`.
6. **Root bucket aggregation:** `handle_merkle_req_bucket()` with path `""` returns bucket hashes aggregated across all partitions via `aggregate_lww_buckets()`.
7. **OR-Map parity:** All 4 OR-Map handlers (`handle_ormap_sync_init`, `handle_ormap_merkle_req_bucket`) follow the same scatter-gather pattern as LWW handlers.
8. **Leaf record fetch unchanged:** Leaf records are still fetched via `hash_to_partition(key)` from the correct `RecordStore`.
9. **Integration tests pass:** All 55 TS-to-Rust integration tests pass without client changes.
10. **Rust tests pass:** All existing Rust tests pass (minus deleted dual-write tests). New tests cover aggregate methods and path prefix routing.

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server 2>&1` -- all pass, no dual-write test failures (they are deleted)
2. Run `pnpm test:integration-rust` -- all 55 tests pass, client sync works without protocol changes
3. Run `pnpm test:k6:smoke` -- verify throughput does not regress (expect improvement under concurrent load)
4. Grep for `CLIENT_SYNC_PARTITION` in server-rust -- zero matches
5. Grep for `partition_id != Self::CLIENT_SYNC_PARTITION` in server-rust -- zero matches

## Constraints

- Do NOT modify any client-side code (TS client is partition-unaware, path is opaque)
- Do NOT modify the wire protocol message schemas (SyncInit, SyncRespRoot, MerkleReqBucket, SyncRespBuckets, SyncRespLeaf)
- Do NOT change `DefaultRecordStore` -- it has no dual-write logic
- Do NOT change `hash_to_partition()` behavior
- Path prefix must be fixed-width (3 digits, zero-padded) to enable unambiguous parsing

## Assumptions

- `wrapping_add` is sufficient for hash aggregation (commutative, associative, same property as Hazelcast's `sumHash`). Collision probability is acceptable for delta sync (false positives cause unnecessary bucket traversal, not data loss).
- 3-digit partition prefix (000-270) is sufficient for 271 partitions. If partition count changes, the prefix width stays 3 digits (supports up to 999).
- DashMap iteration order is non-deterministic but `wrapping_add` is order-independent so this is safe.
- Root-level bucket aggregation (combining `get_buckets("")` across partitions) produces correct results because all trees share the same depth and hex bucket structure.
- The performance improvement is primarily from eliminating the Mutex contention on partition 0, not from reducing the number of hash computations.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add aggregate methods to `MerkleSyncManager` + remove dual-write from `MerkleMutationObserver` + update/delete tests in `merkle_sync.rs` | -- | ~20% |
| G2 | 2 | Update `SyncService` LWW and OR-Map handlers in `sync.rs` to use scatter-gather with path prefix encoding | G1 | ~25% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2 | No | 1 |

**Total workers needed:** 1 (sequential)

## Audit History

### Audit v1 (2026-03-17)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~45% | <=50% | OK |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Audit Dimensions:**
- Clarity: Excellent. Context, task, and requirements are specific and implementable.
- Completeness: All files listed, deletions enumerated (7 tests + constant + code blocks), method signatures provided with doc comments.
- Testability: All 10 acceptance criteria are concrete and verifiable.
- Scope: 2 files, well-bounded by constraints.
- Feasibility: Sound. DashMap v6 supports `.iter()` for aggregate methods; `wrapping_add` is commutative/associative.
- Architecture fit: Aligns with Hazelcast's per-partition Merkle pattern. Eliminates the anti-pattern introduced by SPEC-080.
- Non-duplication: No reinvention.
- Cognitive load: Path encoding adds complexity but is well-documented with clear two-mode convention.
- Strategic fit: Directly addresses measured performance bottleneck (196->126 ops/sec collapse). High value.
- Project compliance: Honors all PROJECT.md decisions. No new dependencies, MsgPack wire format unchanged, proper integer types (u32 for hashes and partition IDs).

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (uses `u32` for hashes, partition IDs)
- [x] No `r#type: String` on message structs (no new message structs)
- [x] `Default` not applicable (no new payload structs with 2+ optional fields)
- [x] Enums used where appropriate (no new string-typed enums)
- [x] Wire compatibility: no serialization changes
- [x] `#[serde(rename_all = "camelCase")]` not applicable (no new structs)
- [x] `#[serde(skip_serializing_if)]` not applicable (no new Option fields)

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (no partition 0) has artifacts | OK | merkle_sync.rs deletions |
| Truth 2 (aggregate root hash) has artifacts | OK | merkle_sync.rs aggregate methods |
| Truth 3 (aggregate buckets) has artifacts | OK | merkle_sync.rs + sync.rs |
| Truth 4 (path prefix routing) has artifacts | OK | sync.rs handlers |
| Truth 5 (leaf fetch unchanged) has artifacts | OK | sync.rs (no change needed) |
| Truth 6 (integration tests) has artifacts | OK | validation checklist |
| All artifacts have purpose | OK | No orphans |
| Key links wired | OK | 3 links all covered |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | wrapping_add is sufficient for hash aggregation | False positives in sync (extra bucket traversal, not data loss) |
| A2 | All partition trees share same depth/structure | Aggregation produces incorrect bucket hashes |
| A3 | Client treats paths as opaque strings | Client rejects partition-prefixed paths |

A1 impact is bounded (performance, not correctness). A2 is guaranteed by MerkleSyncManager using fixed depth. A3 is verified by examining TS client code (paths are opaque).

Strategic fit: Aligned with project goals. Directly unblocks the scaling improvements from SPEC-116/117/118.
Project compliance: Honors PROJECT.md decisions.
Language profile: Compliant with Rust profile (2 files, within 5-file limit).

**Critical issue found and resolved in task groups:**
The original 4-group plan had G1+G2 both modifying `merkle_sync.rs` in parallel (Wave 1) and G3+G4 both modifying `sync.rs` in parallel (Wave 2). This would cause file write conflicts with parallel workers. Restructured to 2 sequential groups: G1 handles all `merkle_sync.rs` changes, G2 handles all `sync.rs` changes.

**Comment:** Well-crafted specification with clear problem statement, concrete method signatures, explicit deletion list, and sound technical approach. Task groups restructured from 4 parallel groups to 2 sequential groups to avoid same-file write conflicts. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-17
**Commits:** 2

### Files Created
None.

### Files Modified
- `packages/server-rust/src/storage/merkle_sync.rs` — Added 6 aggregate/partition-list methods to `MerkleSyncManager`; removed `CLIENT_SYNC_PARTITION` constant and all dual-write blocks from `MerkleMutationObserver::update_tree()`, `on_remove()`, `on_evict()`; deleted 7 dual-write tests; added 12 new tests covering aggregate methods and single-partition write invariant
- `packages/server-rust/src/service/domain/sync.rs` — Updated 4 handlers (`handle_sync_init`, `handle_ormap_sync_init`, `handle_merkle_req_bucket`, `handle_ormap_merkle_req_bucket`) to use scatter-gather; added `parse_partition_prefix()` helper for 3-digit zero-padded partition path routing; added new tests for scatter-gather, path prefix routing, and aggregate bucket requests

### Files Deleted
None (only test functions deleted within existing files, as specified).

### Acceptance Criteria Status
- [x] 1. No dual-write: `update_tree()` writes to exactly one tree. `CLIENT_SYNC_PARTITION` does not exist.
- [x] 2. Aggregate root hash: `aggregate_lww_root_hash("users")` returns `wrapping_add` of all partition hashes. Returns 0 when empty.
- [x] 3. Aggregate root hash is commutative: `wrapping_add` is commutative/associative, order-independent of DashMap iteration.
- [x] 4. SyncService uses aggregate: `handle_sync_init()` calls `aggregate_lww_root_hash()`, not `with_lww_tree(..., 0, ...)`.
- [x] 5. Path prefix routing: `handle_merkle_req_bucket()` with `"042/abc"` routes to partition 42, sub-path `"abc"`.
- [x] 6. Root bucket aggregation: `handle_merkle_req_bucket()` with `""` returns buckets from `aggregate_lww_buckets()`.
- [x] 7. OR-Map parity: all 4 OR-Map handlers follow the same scatter-gather pattern.
- [x] 8. Leaf record fetch unchanged: leaf records still fetched via `hash_to_partition(key)` from `RecordStore`.
- [x] 9. Integration tests pass: 55/55 TS-to-Rust integration tests pass without client changes.
- [x] 10. Rust tests pass: 539 tests pass (7 dual-write tests deleted as specified, 12+ new tests added).

### Deviations
None. Implementation followed the spec exactly.

### Notes
- The `parse_partition_prefix()` helper is module-level (not a method) since it takes no `self` and is used by both LWW and OR-Map handlers.
- Existing `sync.rs` tests that pre-populate partition 0 continue to pass because `aggregate_lww_root_hash()` sums all partitions including 0.
- The `ac7_ormap_push_diff_returns_ack_and_stores_data` test uses an observer at partition 0 as a shared static observer — this remains valid as it tests that `store.put()` fires, not partition routing.

---

## Review History

### Review v1 (2026-03-17)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Major:**

1. **Clippy lint failures in newly added code — 12 errors across both modified files**
   - File: `packages/server-rust/src/storage/merkle_sync.rs:142` (and lines 155, 170, 190, 208, 217)
   - Issue: All 6 new public methods (`aggregate_lww_root_hash`, `aggregate_ormap_root_hash`, `aggregate_lww_buckets`, `aggregate_ormap_buckets`, `lww_partition_ids`, `ormap_partition_ids`) are missing `#[must_use]` attributes. The existing methods on `MerkleSyncManager` (e.g., `with_lww_tree`, `new`) already carry `#[must_use]` — the new methods must follow the same pattern.
   - Fix: Add `#[must_use]` before each `pub fn aggregate_*` and `pub fn *_partition_ids` method signature.

2. **Clippy lint failure — explicit iter loop idiom**
   - File: `packages/server-rust/src/storage/merkle_sync.rs:172` and `:192`
   - Issue: `for entry in self.lww_trees.iter()` and `for entry in self.ormap_trees.iter()` should be `for entry in &self.lww_trees` / `for entry in &self.ormap_trees` per `clippy::explicit_iter_loop`.
   - Fix: Replace `.iter()` with a reference in both `aggregate_lww_buckets` and `aggregate_ormap_buckets` loop bodies.

3. **Clippy lint failure — doc item missing backticks**
   - File: `packages/server-rust/src/storage/merkle_sync.rs:141` and `packages/server-rust/src/service/domain/sync.rs:37`
   - Issue: Doc comment references `wrapping_add` (merkle_sync.rs) and `"042/abc"` (sync.rs) without backtick formatting, triggering `clippy::doc_markdown`.
   - Fix: Wrap the relevant identifiers in backticks in the doc comments.

4. **Clippy lint failure — function too many lines**
   - File: `packages/server-rust/src/service/domain/sync.rs:172` (`handle_merkle_req_bucket`, 133 lines) and `:363` (`handle_ormap_merkle_req_bucket`, 145 lines)
   - Issue: Both handlers exceed the 100-line clippy limit (`clippy::too_many_lines`). The two handlers share near-identical structure (routed mode + aggregate leaf mode + aggregate internal mode).
   - Fix: Extract the shared leaf-collection and record-fetching logic into private helpers (e.g., `collect_lww_leaf_records`, `collect_ormap_leaf_entries`) to bring each handler under 100 lines, or add `#[allow(clippy::too_many_lines)]` with a comment explaining that the complexity is inherent to the two-mode path dispatch.

**Passed:**
- [✓] AC1 (no dual-write) — `CLIENT_SYNC_PARTITION` confirmed absent, `update_tree()` writes only to `self.partition_id`, verified by `single_partition_observer_writes_only_to_its_partition` test
- [✓] AC2 (aggregate root hash returns `wrapping_add`) — `aggregate_lww_root_hash` and `aggregate_ormap_root_hash` implemented correctly; empty case returns 0
- [✓] AC3 (commutativity) — `wrapping_add` is commutative and associative; DashMap order is non-deterministic but safe
- [✓] AC4 (SyncService uses aggregate) — `handle_sync_init` calls `aggregate_lww_root_hash`, no `with_lww_tree(..., 0, ...)` calls; confirmed by grep
- [✓] AC5 (path prefix routing) — `parse_partition_prefix` correctly parses `"042/abc"` → `(42, "abc")`; tested with unit tests including edge cases (empty, short hex, max partition 270)
- [✓] AC6 (root bucket aggregation) — `handle_merkle_req_bucket` with `""` delegates to `aggregate_lww_buckets`
- [✓] AC7 (OR-Map parity) — `handle_ormap_sync_init` and `handle_ormap_merkle_req_bucket` follow the identical scatter-gather pattern
- [✓] AC8 (leaf fetch unchanged) — leaf records fetched via `hash_to_partition(key)` from `RecordStore` in both routed and aggregate modes
- [✓] AC10 (Rust tests pass) — 539 tests pass; 7 dual-write tests confirmed deleted; 12+ new tests added
- [✓] Deletions verified — `CLIENT_SYNC_PARTITION` and `partition_id != Self::CLIENT_SYNC_PARTITION` have zero matches in server-rust
- [✓] 7 deleted dual-write test names have zero matches in server-rust
- [✓] No wire protocol changes — message structs unchanged, MsgPack serialization unchanged
- [✓] No client-side changes — only `merkle_sync.rs` and `sync.rs` modified
- [✓] `parse_partition_prefix` is module-level (not a method) — correct design for a shared pure function
- [✓] Aggregate bucket logic is correct — `combined_buckets.is_empty()` distinguishes leaf nodes from missing data via partition ID check
- [✓] Type correctness — all new methods use `u32` for partition IDs and hashes, `HashMap<char, u32>` for buckets

**Summary:** The implementation correctly meets all 10 acceptance criteria and the functional behavior is sound. However, running `cargo clippy -- -D warnings` produces 12 errors in the newly-added code (missing `#[must_use]`, explicit iter loop idiom, doc-markdown backticks, and two functions exceeding the 100-line limit). The project's existing code uses `#[must_use]` consistently on value-returning public methods, so these are regressions introduced by this spec. All errors are in the two files modified by SPEC-119.

### Fix Response v1 (2026-03-17)
**Applied:** all 4 review issues

**Fixes:**
1. [✓] Missing `#[must_use]` on 6 new public methods — added to all 6 aggregate/partition-list methods
   - Commit: e1986b3
2. [✓] Explicit iterator loop idiom — replaced `.iter()` with `&` prefix in both aggregate_buckets methods
   - Commit: e1986b3
3. [✓] Doc comment missing backticks — wrapped `DashMap` and `sub_path` in backticks
   - Commit: e1986b3
4. [✓] Functions exceed 100-line clippy limit — added `#[allow(clippy::too_many_lines)]` with WHY-comment on both handlers
   - Commit: e1986b3

**Verification:** `cargo clippy -- -D warnings` passes (0 errors). 539 Rust tests pass.

---

### Review v2 (2026-03-17)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1 (no dual-write) — `CLIENT_SYNC_PARTITION` has zero matches in server-rust; `update_tree()` writes only to `self.partition_id`; `single_partition_observer_writes_only_to_its_partition` test verifies single-partition invariant
- [✓] AC2 (aggregate root hash) — `aggregate_lww_root_hash` and `aggregate_ormap_root_hash` implemented correctly; empty returns 0; combines via `wrapping_add`
- [✓] AC3 (commutativity) — `wrapping_add` is commutative/associative; DashMap iteration order is non-deterministic but safe
- [✓] AC4 (SyncService uses aggregate) — `handle_sync_init` calls `aggregate_lww_root_hash`; no `with_lww_tree(..., 0, ...)` in production handlers
- [✓] AC5 (path prefix routing) — `parse_partition_prefix` correctly parses `"042/abc"` → `(42, "abc")`
- [✓] AC6 (root bucket aggregation) — `handle_merkle_req_bucket` with `""` delegates to `aggregate_lww_buckets`
- [✓] AC7 (OR-Map parity) — `handle_ormap_sync_init` and `handle_ormap_merkle_req_bucket` follow identical scatter-gather pattern
- [✓] AC8 (leaf fetch unchanged) — leaf records fetched via `hash_to_partition(key)` from `RecordStore` in both modes
- [✓] AC10 (Rust tests pass) — 539/539 tests pass; 7 dual-write tests confirmed deleted; 12+ new aggregate/routing tests added
- [✓] Fix v1 Issue 1 verified — all 6 new public methods carry `#[must_use]` at lines 142, 156, 172, 193, 212, 222 in merkle_sync.rs
- [✓] Fix v1 Issue 2 verified — `for entry in &self.lww_trees` (line 175) and `for entry in &self.ormap_trees` (line 195); no explicit `.iter()` in loop heads
- [✓] Fix v1 Issue 3 verified — doc comments use backticks for `wrapping_add`, `DashMap`, `sub_path` in both files
- [✓] Fix v1 Issue 4 verified — `#[allow(clippy::too_many_lines)]` with WHY-comment on both handlers at lines 172 and 364 in sync.rs
- [✓] `cargo clippy -p topgun-server -- -D warnings` passes with zero errors or warnings
- [✓] Deletions verified — `CLIENT_SYNC_PARTITION` and `partition_id != Self::CLIENT_SYNC_PARTITION` have zero matches in server-rust
- [✓] 7 deleted dual-write test names have zero matches in server-rust
- [✓] No wire protocol changes — message structs unchanged, MsgPack serialization unchanged
- [✓] No client-side changes — only `merkle_sync.rs` and `sync.rs` modified
- [✓] `parse_partition_prefix` is module-level function — appropriate design for a shared pure helper
- [✓] Type correctness — all new methods use `u32` for hashes and partition IDs, `HashMap<char, u32>` for buckets
- [✓] Architecture alignment — per-partition Merkle trees with scatter-gather matches Hazelcast pattern; eliminates anti-pattern from SPEC-080

**Summary:** All 4 issues from Review v1 are confirmed fixed. Clippy passes clean, 539 Rust tests pass, all 10 acceptance criteria are met, and all specified deletions are verified. The implementation is clean, well-tested, and architecturally sound.

---

## Completion

**Completed:** 2026-03-17
**Total Commits:** 3
**Review Cycles:** 2

### Outcome

Eliminated the partition 0 Mutex bottleneck by replacing dual-write with scatter-gather root hash aggregation, enabling per-partition write parallelism to scale linearly under concurrent load.

### Key Files

- `packages/server-rust/src/storage/merkle_sync.rs` — Aggregate methods on MerkleSyncManager; simplified MerkleMutationObserver to single-partition writes
- `packages/server-rust/src/service/domain/sync.rs` — Scatter-gather handlers with path prefix routing for partition-aware Merkle traversal

### Patterns Established

- **Scatter-gather aggregation pattern:** Compute aggregate values at query time by iterating per-partition data structures via `wrapping_add`, rather than maintaining a global aggregate on the write path.
- **3-digit zero-padded path prefix routing:** `"042/abc"` convention for encoding partition ID in opaque path strings, enabling routed vs aggregate mode dispatch.

### Deviations

None — implemented as specified.
