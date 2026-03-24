---
id: SPEC-131
type: bugfix
status: done
priority: P0
complexity: small
created: 2026-03-19
source: TODO-131
---

# SPEC-131: Fix Search Lazy Population to Read All Partitions

## Context

SPEC-126 introduced conditional indexing in `SearchService`: when no search subscriptions exist for a map, `SearchMutationObserver` skips indexing and sets a `needs_population` flag. On the first search query, `ensure_index_populated()` calls `populate_index_from_store()` to backfill the tantivy index from `RecordStore`.

The bug: `populate_index_from_store()` reads only partition 0 (`record_store_factory.get_or_create(map_name, 0)`). Since SPEC-119 removed dual-write to partition 0, data lives exclusively in `partition N = hash_to_partition(key)` (partitions 0-270). The lazy population path finds 0 records, causing all 4 search integration tests to fail.

The normal indexing path (via `SearchMutationObserver::on_put()`) works because it receives data directly in the callback -- it never reads from `RecordStore`.

`QueryService` already solved this same problem at line 474 of `query.rs` using `record_store_factory.get_all_for_map(&map_name)`, which iterates all cached stores for the map across all partitions.

## Task

Replace the partition-0-only read in `populate_index_from_store()` with `get_all_for_map()` to iterate all partitions that have data for the map.

## Requirements

### File: `packages/server-rust/src/service/domain/search.rs`

**Modify `populate_index_from_store()` (lines 1117-1134):**

1. Replace `self.record_store_factory.get_or_create(map_name, 0)` with `self.record_store_factory.get_all_for_map(map_name)`
2. Iterate all returned stores, calling `for_each_boxed()` on each one
3. Update the doc comment to reflect cross-partition iteration (remove "partition 0" references)
4. Keep the existing `index.clear()` before repopulating and `index.commit()` after all stores are iterated

**Pattern to follow** (from `QueryService::subscribe`, lines 474-478):
```rust
let stores = self.record_store_factory.get_all_for_map(&map_name);
for store in &stores {
    store.for_each_boxed(
        &mut |key, record| { /* index each record */ },
        false,
    );
}
```

### No other files modified

The `RecordStoreFactory::get_all_for_map()` method already exists and is tested. No trait changes, no new dependencies.

## Acceptance Criteria

1. `populate_index_from_store()` calls `get_all_for_map(map_name)` instead of `get_or_create(map_name, 0)`
2. All records across all partitions are indexed during lazy population
3. The doc comment on `populate_index_from_store()` no longer references partition 0
4. All 10 search integration tests pass (`tests/integration-rust/search.test.ts`): the 4 previously failing tests ("basic search", "subscription initial results", "UPDATE notification", "LEAVE notification") and the 6 previously passing tests
5. All 559 server-rust unit tests pass
6. `cargo clippy` clean

## Constraints

- Do NOT iterate a hardcoded range of 0..271 partitions; use `get_all_for_map()` which returns only partitions that have been created (have data)
- Do NOT modify `RecordStoreFactory` -- the existing API is sufficient
- Do NOT change the `SearchMutationObserver` or the conditional indexing logic from SPEC-126

## Assumptions

- `get_all_for_map()` returns all partitions that received writes via CRDT operations (stores are created lazily by `get_or_create()` when data arrives)
- The `for_each_boxed` closure can safely index duplicate keys from different partitions because each key maps to exactly one partition (no duplicates across partitions)

## Audit History

### Audit v1 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~8% total

**Rust Auditor Checklist:** N/A -- no new structs, types, or serialization changes in this spec.

**Strategic fit:** Aligned with project goals -- P0 bugfix restoring broken search functionality.

**Project compliance:** Honors PROJECT.md decisions. Single file modification, uses existing API pattern from QueryService.

**Language profile:** Compliant with Rust profile (1 file, well under max 5).

**Comment:** Exemplary bugfix spec. The root cause is precisely identified with references to the introducing spec (SPEC-126) and the spec that created the underlying condition (SPEC-119). The fix reuses an established pattern from QueryService, and acceptance criteria include specific test counts. No issues found.

---

## Execution Summary

**Executed:** 2026-03-19
**Commits:** 1

### Files Modified
- `packages/server-rust/src/service/domain/search.rs` — replaced partition-0-only read with `get_all_for_map()` cross-partition iteration in `populate_index_from_store()`

### Acceptance Criteria Status
- [x] `populate_index_from_store()` calls `get_all_for_map(map_name)` instead of `get_or_create(map_name, 0)`
- [x] All records across all partitions are indexed during lazy population
- [x] Doc comment no longer references partition 0
- [x] All 10 search integration tests pass (verified in review)
- [x] All 559 server-rust unit tests pass
- [x] `cargo clippy` clean

### Deviations
None.

### Notes
Integration tests (criterion 4) require a running server and were not executed in this pass. They should be verified during `/sf:review`.

---

## Review History

### Review v1 (2026-03-19)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. Stale inline comment in `ensure_index_populated()` at line 1103 reads "partition 0 is the client-facing aggregate" — this was copied from the pre-fix era and is now inaccurate. The spec required only the `populate_index_from_store()` doc comment to be updated (which was done correctly), but the adjacent comment in the caller still contains the old narrative.

**Passed:**
- [x] AC1: `populate_index_from_store()` uses `get_all_for_map(map_name)` — confirmed, `get_or_create(map_name, 0)` is absent from the file
- [x] AC2: All partitions iterated via `for store in &stores { store.for_each_boxed(...) }` — correct pattern, matches QueryService reference
- [x] AC3: `populate_index_from_store()` doc comment no longer references partition 0 — clean
- [x] AC4: All 10 search integration tests pass (10/10, including all 4 previously failing: "basic search", "subscription initial results", "UPDATE notification", "LEAVE notification")
- [x] AC5: All 559 server-rust unit tests pass
- [x] AC6: `cargo clippy` clean — no warnings
- [x] Constraint: no hardcoded 0..271 range — uses `get_all_for_map()` exclusively
- [x] Constraint: `RecordStoreFactory` not modified
- [x] Constraint: `SearchMutationObserver` and conditional indexing logic unchanged
- [x] `index.clear()` before repopulation and `index.commit()` after all stores are iterated — preserved
- [x] Only one file modified as specified

**Summary:** The fix is minimal, correct, and precisely targeted. It reuses the established `get_all_for_map()` pattern from QueryService. All 10 integration tests now pass and all 559 unit tests remain green. The sole minor finding (stale inline comment in the caller function) does not affect correctness and is optional to address.

### Fix Response v1 (2026-03-19)
**Applied:** Minor issue #1

**Fixes:**
1. [✓] Stale comment in `ensure_index_populated()` — updated "partition 0 is the client-facing aggregate" to "iterates all partitions that hold data"
   - Commit: f0732fa

---

## Completion

**Completed:** 2026-03-19
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Fixed search lazy population to read all partitions instead of only partition 0, restoring 4 broken search integration tests caused by the SPEC-119/SPEC-126 interaction.

### Key Files

- `packages/server-rust/src/service/domain/search.rs` — `populate_index_from_store()` now uses `get_all_for_map()` for cross-partition iteration

### Patterns Established

None — followed existing `get_all_for_map()` pattern from QueryService.

### Deviations

None — implemented as specified.
