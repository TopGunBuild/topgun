---
id: SPEC-080
type: bugfix
status: done
priority: P0
complexity: medium
created: 2026-03-07
todo: TODO-111
---

# SPEC-080: Fix Merkle Sync Partition Mismatch (Late-Joiner Bug)

## Context

Late-joining clients never receive existing data via Merkle sync. The root cause is a partition routing mismatch between writes and sync lookups:

- **Writes:** `CrdtService` stores records at `hash_to_partition(record_key)` (e.g., partition 42 for key "alice")
- **Sync lookups:** `SyncService` reads Merkle trees at `hash_to_partition(map_name)` (e.g., partition 123 for map "users")

Since `hash_to_partition("alice") != hash_to_partition("users")`, the Merkle tree at the sync partition is always empty. Late-joiners receive `rootHash: 0` and conclude no data exists.

The TS server does NOT use partitions for client-facing Merkle sync. Each `LWWMap` has a single `MerkleTree` (not per-partition). Partitions exist only for cluster replication/repair, not client sync.

The existing integration test `merkle-sync.test.ts` masks the bug by using key `"u55"`, which happens to hash to the same partition (123) as map name `"users"`.

**There are THREE distinct problems to fix:**

1. **Classify mismatch:** `classify.rs` sets `partition_key = map_name` for sync messages, producing the wrong partition ID.
2. **Observer single-write:** `MerkleMutationObserver` only updates the Merkle tree at the record's actual partition. Since SyncService reads at partition 0 (when classify passes None), the client-sync tree is empty.
3. **Record lookup mismatch:** SyncService's leaf/diff handlers use `get_or_create(map_name, partition_id)` to fetch records. With partition_id=0, records stored at `hash_to_partition(key)` are not found. These handlers must search across all partitions.

## Task

Fix the partition mismatch by establishing partition 0 as the "client sync" partition for all Merkle tree operations, while keeping actual record storage at `hash_to_partition(key)` for future cluster use.

## Goal Analysis

**Goal Statement:** Late-joining clients receive existing data via Merkle delta sync, regardless of which partition stores the records.

**Observable Truths:**
1. A late-joining client sending `SYNC_INIT` for map "users" receives a non-zero `rootHash` when data exists, regardless of which keys are stored
2. A late-joining client completing the full Merkle sync protocol (SYNC_INIT -> MERKLE_REQ_BUCKET -> leaf records) receives all records for the map
3. The Merkle tree at partition 0 contains entries for ALL records in the map, not just records that happen to hash to partition 0
4. Records continue to be stored at `hash_to_partition(key)` (no change to record storage)
5. Existing unit tests continue to pass (no regression)

**Required Artifacts:**
- `classify.rs` -- sync messages produce `partition_id: None` (defaults to 0 in SyncService)
- `merkle_sync.rs` -- `MerkleMutationObserver` dual-writes to both actual partition and partition 0
- `sync.rs` -- leaf/diff record lookups search across all partitions via `get_all_for_map()`
- `merkle-sync.test.ts` -- validates fix with arbitrary key (not partition-aligned)

**Key Links:**
- classify.rs -> sync.rs: `ctx.partition_id` flows from classify to SyncService handlers. Setting it to None makes SyncService use 0.
- merkle_sync.rs -> sync.rs: Observer writes to partition 0 tree; SyncService reads from partition 0 tree. These must agree.
- sync.rs record lookup: Merkle tree keys reference records across ALL partitions. Leaf handler must search all partitions, not just partition 0.

## Requirements

### File 1: `packages/server-rust/src/service/classify.rs`

**Change:** Remove `partition_key` from all 6 sync message classifications.

For these 6 message types, change `partition_key` from `Some(payload.map_name.as_str())` or `Some(payload.payload.map_name.as_str())` to `None`:

1. `Message::SyncInit` (line 110)
2. `Message::MerkleReqBucket` (line 120)
3. `Message::ORMapSyncInit` (line 130)
4. `Message::ORMapMerkleReqBucket` (line 140)
5. `Message::ORMapDiffRequest` (line 150)
6. `Message::ORMapPushDiff` (line 160)

This causes `ctx.partition_id` to be `None`, which SyncService already handles via `ctx.partition_id.unwrap_or(0)`.

**Update existing test:** `classify_client_op` test at line 576 asserts `partition_id.is_some()` for ClientOp -- this remains correct. Add a new test verifying SyncInit produces `partition_id: None`.

### File 2: `packages/server-rust/src/storage/merkle_sync.rs`

**Change:** `MerkleMutationObserver` must dual-write to both the record's actual partition AND partition 0 (the "client sync" partition).

In `update_tree()`, after updating the tree at `self.partition_id`, also update partition 0 if `self.partition_id != 0`. Same for all remove operations (`on_remove`, `on_evict`). Same for `on_clear`/`on_reset`/`on_destroy` -- these clear only the observer's own partition, NOT partition 0 (partition 0 is an aggregate and should not be cleared by a single partition's lifecycle).

Specific changes to `update_tree()`:
```
fn update_tree(&self, key: &str, value: &RecordValue) {
    // ... existing match on value, updating self.partition_id ...

    // Dual-write: also update partition 0 (client sync tree)
    if self.partition_id != 0 {
        // repeat the same update/remove call with partition_id = 0
    }
}
```

Same pattern for `on_remove` and `on_evict`: after removing from `self.partition_id`, also remove from partition 0 if `self.partition_id != 0`.

**Important:** `on_clear`, `on_reset`, `on_destroy` must NOT clear partition 0 when `self.partition_id != 0`. Partition 0 is a cross-partition aggregate; clearing it when one partition resets would lose data from other partitions.

**Note on `on_replication_put`:** This method does NOT check `is_backup` (by design). The dual-write to partition 0 should also apply here.

### File 3: `packages/server-rust/src/service/domain/sync.rs`

**Change:** Leaf and diff record lookups must search across all partitions, not just partition 0.

In `handle_merkle_req_bucket()` (line 174): Replace `self.record_store_factory.get_or_create(&map_name, partition_id)` with a cross-partition lookup. For each key from the Merkle tree leaf, use `hash_to_partition(key)` to find the correct partition, then `get_or_create(&map_name, computed_partition)` to fetch from the right store.

Same change needed in:
- `handle_ormap_merkle_req_bucket()` (line 281) -- OR-Map leaf record fetch
- `handle_ormap_diff_request()` (line 354) -- OR-Map diff record fetch

For `handle_ormap_push_diff()` (line 433): This writes records, so it should use `hash_to_partition(entry.key)` instead of the sync partition. Each entry's key determines its storage partition.

**Approach:** Import `topgun_core::hash_to_partition` in sync.rs. For each key in a leaf/diff request, compute `hash_to_partition(&key)` and fetch from that partition's store. This avoids loading ALL stores via `get_all_for_map()` and is O(keys) not O(partitions).

### File 4: `tests/integration-rust/merkle-sync.test.ts`

**Change:** Replace the partition-aligned key `"u55"` with an arbitrary key (e.g., `"alice"`) to prove the fix works for any key. Remove the comment at lines 27-30 explaining the partition alignment hack.

## Acceptance Criteria

1. **AC1:** `classify.rs` produces `partition_id: None` for all 6 sync message types (SyncInit, MerkleReqBucket, ORMapSyncInit, ORMapMerkleReqBucket, ORMapDiffRequest, ORMapPushDiff)
2. **AC2:** `MerkleMutationObserver.update_tree()` updates both `self.partition_id` and partition 0 when `self.partition_id != 0`
3. **AC3:** `MerkleMutationObserver.on_remove()` and `on_evict()` remove from both `self.partition_id` and partition 0 when `self.partition_id != 0`
4. **AC4:** `MerkleMutationObserver.on_clear()`/`on_reset()`/`on_destroy()` do NOT clear partition 0 when `self.partition_id != 0`
5. **AC5:** SyncService leaf handlers (`handle_merkle_req_bucket`, `handle_ormap_merkle_req_bucket`) fetch records from `hash_to_partition(key)` partition, not from partition 0
6. **AC6:** SyncService diff/push handlers (`handle_ormap_diff_request`, `handle_ormap_push_diff`) fetch/store records at `hash_to_partition(key)` partition
7. **AC7:** Integration test `merkle-sync.test.ts` uses a non-partition-aligned key and passes
8. **AC8:** All existing Rust unit tests pass (`cargo test --release -p topgun-server`)
9. **AC9:** All existing integration tests pass (`pnpm test:integration-rust`)

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server 2>&1` -- all tests pass, including new classify test for SyncInit partition_id=None
2. Run `pnpm test:integration-rust` -- merkle-sync test passes with arbitrary key (not "u55")
3. In `classify.rs`, verify all 6 sync message arms pass `None` as `partition_key` to `make_ctx`
4. In `merkle_sync.rs`, verify `update_tree()` calls both `self.partition_id` and `0` for every Merkle update
5. In `sync.rs`, verify leaf handlers compute `hash_to_partition(&key)` per-key instead of using `partition_id` from context

## Constraints

- Do NOT change how `CrdtService` stores records -- records must stay at `hash_to_partition(key)` for correct cluster partitioning
- Do NOT remove the per-partition Merkle trees -- they will be needed for cluster replication/repair in the future
- Do NOT change partition 0's meaning for RecordStore -- partition 0 is only special for Merkle trees (client sync), not for record storage
- Do NOT modify `MerkleSyncManager`'s API -- changes are confined to the observer and its callers
- Do NOT add `hash_to_partition` calls to `MerkleMutationObserver` -- the observer already knows its `partition_id` at construction time; the dual-write just targets a fixed constant (0)

## Assumptions

- Partition 0 as the "client sync" partition ID is acceptable (no records happen to need partition 0 exclusively; it's one of 271 partitions and records already hash into it naturally)
- The dual-write approach (actual partition + partition 0) has negligible performance impact since Merkle tree updates are in-memory hash operations
- `get_all_for_map()` is NOT used in SyncService -- per-key `hash_to_partition` lookup is preferred for efficiency
- OR-Map sync handlers follow the same partition fix pattern as LWW handlers
- No changes needed to `CrdtService` (crdt.rs) since it already stores at the correct partition; it just needs the observer to dual-write

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Remove partition_key from 6 sync arms in classify.rs; add unit test for SyncInit partition_id=None | -- | ~10% |
| G2 | 1 | Add dual-write logic to MerkleMutationObserver (partition 0 + actual partition) in merkle_sync.rs; add unit tests for dual-write behavior | -- | ~25% |
| G3 | 2 | Fix SyncService leaf/diff/push handlers to use per-key hash_to_partition for record lookup in sync.rs | G1, G2 | ~25% |
| G4 | 3 | Update merkle-sync.test.ts to use arbitrary key; run full test suite | G3 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

**Note on trait-first:** This spec is a bugfix modifying existing implementations. No new traits or types are introduced, so the trait-first requirement does not apply. G1 and G2 are independent implementation changes that can proceed in parallel.

## Audit History

### Audit v1 (2026-03-07)
**Status:** APPROVED

**Context Estimate:** ~70% total (but ~15-30% per worker with orchestrated execution)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~70% | <=50% | -- |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~5% per worker | <=10% | OK |

**Quality Projection (per worker):**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | -- |
| 30-50% | GOOD | <-- All workers land here or better |
| 50-70% | DEGRADING | -- |
| 70%+ | POOR | -- |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Per-Worker |
|-------|------|-------|--------------|------------|
| G1 | 1 | classify.rs fix + test | ~10% | ~15% |
| G2 | 1 | merkle_sync.rs dual-write + tests | ~25% | ~30% |
| G3 | 2 | sync.rs per-key lookup fix | ~25% | ~30% |
| G4 | 3 | integration test update | ~10% | ~15% |

**Dimensions:**
- Clarity: PASS -- Three distinct problems clearly identified with precise code locations
- Completeness: PASS -- All 4 files specified, line numbers verified against source
- Testability: PASS -- 9 ACs, all concrete and measurable
- Scope: PASS -- 4 files within 5-file Language Profile limit
- Feasibility: PASS -- `hash_to_partition` is public in `topgun_core`, approach is sound
- Architecture fit: PASS -- Uses existing observer/factory/manager patterns
- Non-duplication: PASS -- Fixes existing code, no new abstractions
- Cognitive load: PASS -- Three targeted changes, logically independent
- Strategic fit: PASS -- P0 bugfix for critical late-joiner sync failure
- Project compliance: PASS -- No new dependencies, follows Rust conventions
- Language profile: PASS -- 4 files (max 5), trait-first N/A (bugfix), no new traits

**Goal-Backward Validation:** All 5 truths have artifacts. All artifacts map to truths. Key links identified. No orphans.

**Strategic fit:** Aligned with project goals

**Project compliance:** Honors PROJECT.md decisions

**Language profile:** Compliant with Rust profile

**Recommendations:**
1. Minor text inconsistency: Goal Analysis "Required Artifacts" says sync.rs uses `get_all_for_map()`, but Requirements section and Assumptions explicitly say per-key `hash_to_partition` is preferred instead. Consider updating the Goal Analysis line to match: "leaf/diff record lookups use per-key `hash_to_partition()`".

**Comment:** Excellent spec quality. The three-problem decomposition is clear and well-justified. Code locations verified against actual source. The dual-write approach (partition 0 as client-sync aggregate) is a clean solution that preserves future cluster partition use. Task grouping with G1/G2 parallel in Wave 1 is efficient.

## Execution Summary

**Executed:** 2026-03-07
**Mode:** orchestrated
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |
| 3 | G4 | complete |

### Files Modified
- `packages/server-rust/src/service/classify.rs` -- removed partition_key from 6 sync message arms
- `packages/server-rust/src/storage/merkle_sync.rs` -- added dual-write to partition 0 in MerkleMutationObserver
- `packages/server-rust/src/service/domain/sync.rs` -- per-key hash_to_partition for leaf/diff/push record lookups
- `tests/integration-rust/merkle-sync.test.ts` -- replaced "u55" with "alice" (arbitrary key)

### Acceptance Criteria Status
- [x] AC1: classify.rs produces partition_id: None for all 6 sync message types
- [x] AC2: MerkleMutationObserver.update_tree() updates both self.partition_id and partition 0
- [x] AC3: MerkleMutationObserver.on_remove() and on_evict() remove from both partitions
- [x] AC4: on_clear/on_reset/on_destroy do NOT clear partition 0 for non-zero partitions
- [x] AC5: SyncService leaf handlers use hash_to_partition(key) for record lookup
- [x] AC6: SyncService diff/push handlers use hash_to_partition(key) for record lookup
- [x] AC7: Integration test uses non-partition-aligned key ("alice") and passes
- [x] AC8: All 509 Rust unit tests pass
- [x] AC9: All 51 integration tests pass (7 suites)

### Deviations
None.

---

## Review History

### Review v1 (2026-03-07 13:35)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [v] AC1: All 6 sync message types in `classify.rs` (SyncInit, MerkleReqBucket, ORMapSyncInit, ORMapMerkleReqBucket, ORMapDiffRequest, ORMapPushDiff) pass `None` as `partition_key`. Three new unit tests verify SyncInit, MerkleReqBucket, and ORMapSyncInit produce `partition_id: None`.
- [v] AC2: `update_tree()` in `merkle_sync.rs` (lines 236-267) dual-writes to both `self.partition_id` and `CLIENT_SYNC_PARTITION` (0) for all three `RecordValue` variants (Lww, OrMap, OrTombstones). Guard `if self.partition_id != Self::CLIENT_SYNC_PARTITION` prevents double-write when already at partition 0.
- [v] AC3: `on_remove()` (lines 295-312) and `on_evict()` (lines 315-331) both remove from `self.partition_id` then from partition 0 when `self.partition_id != 0`. Both LWW and OR-Map trees are cleaned in both methods. Unit tests `dual_write_remove_clears_from_partition_0` and `dual_write_evict_clears_from_partition_0` verify this.
- [v] AC4: `on_clear()`, `on_reset()`, `on_destroy()` (lines 347-360) all call `clear_partition` only with `self.partition_id`, never touching partition 0. Unit test `on_clear_does_not_clear_partition_0_for_non_zero_partition` explicitly verifies partition 0 retains data after a non-zero partition clears.
- [v] AC5: `handle_merkle_req_bucket()` (sync.rs line 179) and `handle_ormap_merkle_req_bucket()` (line 288) both compute `hash_to_partition(&key)` per-key inside the leaf branch, fetching from the correct storage partition.
- [v] AC6: `handle_ormap_diff_request()` (line 363) and `handle_ormap_push_diff()` (line 441) both use `hash_to_partition(&key)` / `hash_to_partition(&entry.key)` for per-key record fetch/store.
- [v] AC7: Integration test `merkle-sync.test.ts` uses key `"alice"` (line 39), not `"u55"`. No references to `"u55"` remain. Test passes.
- [v] AC8: All 509 Rust unit tests pass (0 failures, clippy clean with `-D warnings`).
- [v] AC9: All 51 integration tests pass across 7 suites.

**Quality Assessment:**
- Code is clean and idiomatic Rust. The `CLIENT_SYNC_PARTITION` constant (line 229) avoids magic numbers.
- `on_replication_put` correctly dual-writes without checking `is_backup`, as specified.
- No unnecessary `.clone()` calls. Borrows are used appropriately.
- Error handling in sync.rs is appropriate -- store errors produce empty entries rather than failing the entire request.
- No security concerns -- no new inputs, no new attack surface.
- Architecture follows existing observer/factory/manager patterns exactly.
- No duplication -- dual-write logic is centralized in `update_tree()` for puts/updates/loads, with parallel logic in `on_remove`/`on_evict` for removals.
- Cognitive load is low -- the `CLIENT_SYNC_PARTITION` constant and clear comments make the dual-write intent obvious.
- All 5 constraints honored: CrdtService unchanged, per-partition trees preserved, partition 0 meaning unchanged for RecordStore, MerkleSyncManager API unchanged, no `hash_to_partition` in observer.

**Summary:** Implementation is complete, correct, and well-tested. All 9 acceptance criteria are met with no deviations. The three-pronged fix (classify, observer, sync service) is clean and follows the spec precisely. Test coverage includes both unit tests for the dual-write behavior and an end-to-end integration test with an arbitrary key. No issues found.

---

## Completion

**Completed:** 2026-03-07
**Total Commits:** 3
**Review Cycles:** 1

### Outcome

Fixed the P0 Merkle sync partition mismatch bug that prevented late-joining clients from receiving existing data. Established partition 0 as the "client sync" aggregate Merkle tree via dual-write in the observer, while preserving per-key record storage for future cluster use.

### Key Files

- `packages/server-rust/src/service/classify.rs` — sync messages now produce partition_id: None (defaults to 0)
- `packages/server-rust/src/storage/merkle_sync.rs` — MerkleMutationObserver dual-writes to partition 0 for client sync
- `packages/server-rust/src/service/domain/sync.rs` — per-key hash_to_partition for correct cross-partition record lookup
- `tests/integration-rust/merkle-sync.test.ts` — validates fix with arbitrary key ("alice")

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
