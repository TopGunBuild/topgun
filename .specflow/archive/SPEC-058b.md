---
id: SPEC-058b
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-21
parent: SPEC-058
depends_on: [SPEC-058a]
todo_ref: TODO-067
---

# Storage In-Memory Implementations (HashMapStorage + NullDataStore)

## Context

This is the second sub-spec of SPEC-058 (Multi-Layer Storage System). It delivers the concrete in-memory implementations of the Layer 1 and Layer 3 traits defined in SPEC-058a:

- **HashMapStorage:** DashMap-backed `StorageEngine` implementation for zero-latency in-memory key-value access with cursor-based iteration and eviction sampling
- **NullDataStore:** No-op `MapDataStore` for testing and ephemeral data scenarios

These are the Wave 2 implementations that can be built and tested independently against the traits from SPEC-058a. They have no dependency on each other and no dependency on `DefaultRecordStore` (SPEC-058c).

### Design Source

- `HashMapStorage`: RUST_STORAGE_ARCHITECTURE.md section 3.3
- `NullDataStore`: RUST_STORAGE_ARCHITECTURE.md section 6.4
- Parent spec SPEC-058 Implementation Details section

### Key Links

- `HashMapStorage` implements `StorageEngine` from `crate::storage::engine`
- `NullDataStore` implements `MapDataStore` from `crate::storage::map_data_store`
- Both use `Record` and `RecordValue` from `crate::storage::record`
- `HashMapStorage::random_samples()` requires the `rand` crate (new dependency)

## Task

Create `engines/` and `datastores/` sub-directories under `packages/server-rust/src/storage/` with `HashMapStorage` and `NullDataStore` implementations, comprehensive unit tests, and the `rand` dependency.

## Requirements

### Files to Create

```
packages/server-rust/src/storage/
  engines/
    mod.rs                  # Re-export HashMapStorage
    hashmap.rs              # HashMapStorage (DashMap-backed StorageEngine)
  datastores/
    mod.rs                  # Re-export NullDataStore
    null.rs                 # NullDataStore (no-op MapDataStore)
```

### Files to Modify

- `packages/server-rust/Cargo.toml` -- add `rand = "0.9"` (modified by G1)
- `packages/server-rust/src/storage/mod.rs` -- add `pub mod engines;` and `pub mod datastores;` declarations + re-exports (modified by G1 only; G2 does not touch this file)

**Total: 4 new + 2 modified = 6 file touches.** Both modifications are small (1-2 lines each for `mod.rs`; 1 line for `Cargo.toml`). Same precedent as SPEC-057a (6 touches, approved).

### Implementation: `HashMapStorage` (`engines/hashmap.rs`)

```rust
use dashmap::DashMap;

/// In-memory storage backed by DashMap for concurrent read access.
pub struct HashMapStorage {
    entries: DashMap<String, Record>,
}
```

**Constructor:**
- `new() -> Self` -- creates empty DashMap

**StorageEngine implementation:**

- `put(&self, key, record)` -- `self.entries.insert(key.to_string(), record)` returns `Option<Record>`
- `get(&self, key)` -- `self.entries.get(key).map(|r| r.clone())`
- `remove(&self, key)` -- `self.entries.remove(key).map(|(_, r)| r)`
- `contains_key(&self, key)` -- `self.entries.contains_key(key)`
- `len(&self)` -- `self.entries.len()`
- `is_empty(&self)` -- `self.entries.is_empty()`
- `clear(&self)` -- `self.entries.clear()` (DashMap::clear takes `&self`)
- `destroy(&self)` -- calls `self.clear()`
- `estimated_cost(&self)` -- iterates all entries, sums `record.metadata.cost`
- `snapshot_iter(&self)` -- collects all entries into `Vec<(String, Record)>` via DashMap iteration (point-in-time snapshot)
- `random_samples(&self, sample_count)` -- uses reservoir sampling: iterate all entries, for each entry at index `i`, if `i < sample_count` add to result, else replace a random existing sample with probability `sample_count / (i + 1)`. Uses `rand::thread_rng()`. Returns at most `min(sample_count, len())` entries.
- `fetch_keys(&self, cursor, size)` -- takes a snapshot via `snapshot_iter()`, decodes cursor state as `u64` offset (little-endian, empty = 0), skips `offset` entries, takes `size`, returns new cursor with updated offset. Sets `finished = true` when `offset + size >= total`.
- `fetch_entries(&self, cursor, size)` -- same cursor logic as `fetch_keys`, returns `(String, Record)` tuples.

**Cursor encoding:** The cursor `state` field stores a `u64` offset as 8 little-endian bytes. An empty `state` (from `IterationCursor::start()`) is treated as offset 0.

### Implementation: `NullDataStore` (`datastores/null.rs`)

```rust
/// No-op MapDataStore for testing and ephemeral data.
/// All operations succeed immediately without side effects.
pub struct NullDataStore;
```

**MapDataStore implementation:**

- `add()` -- returns `Ok(())`
- `add_backup()` -- returns `Ok(())`
- `remove()` -- returns `Ok(())`
- `remove_backup()` -- returns `Ok(())`
- `load()` -- returns `Ok(None)`
- `load_all()` -- returns `Ok(Vec::new())`
- `remove_all()` -- returns `Ok(())`
- `is_loadable()` -- returns `true`
- `pending_operation_count()` -- returns `0`
- `soft_flush()` -- returns `Ok(0)`
- `hard_flush()` -- returns `Ok(())`
- `flush_key()` -- returns `Ok(())`
- `reset()` -- no-op
- `is_null()` -- returns `true`

### Sub-module re-exports

`storage/engines/mod.rs`:
```rust
mod hashmap;
pub use hashmap::HashMapStorage;
```

`storage/datastores/mod.rs`:
```rust
mod null;
pub use null::NullDataStore;
```

Update `storage/mod.rs` (done by G1) to add:
```rust
pub mod engines;
pub mod datastores;

pub use engines::*;
pub use datastores::*;
```

### Dependencies

Add to `packages/server-rust/Cargo.toml` under `[dependencies]`:
```toml
rand = "0.9"
```

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero warnings
2. `cargo clippy -p topgun-server -- -D warnings` passes clean
3. `HashMapStorage` unit tests verify:
   - `put`/`get`/`remove` round-trip
   - `contains_key` returns true after put, false after remove
   - `len`/`is_empty` reflect current state
   - `clear` empties the storage
   - `fetch_keys` with cursor pagination: first page returns correct keys, second page returns remaining, finished cursor at end
   - `fetch_entries` with cursor pagination: same cursor logic as `fetch_keys`
   - `snapshot_iter` returns all entries
   - `random_samples` returns at most `sample_count` entries, returns 0 for empty storage
   - `estimated_cost` reflects sum of `record.metadata.cost` across all stored records
4. `NullDataStore` unit tests verify:
   - All async methods return `Ok`
   - `load()` returns `Ok(None)`
   - `load_all()` returns `Ok(Vec::new())`
   - `is_null()` returns `true`
   - `is_loadable()` returns `true`
   - `pending_operation_count()` returns `0`
   - `soft_flush()` returns `Ok(0)`
5. `HashMapStorage` and `NullDataStore` are accessible via `crate::storage::HashMapStorage` and `crate::storage::NullDataStore` re-exports

## Constraints

- Do NOT modify any traits defined in SPEC-058a
- Do NOT add PostgreSQL/sqlx dependencies
- Do NOT use `f64` for any integer-semantic field
- `HashMapStorage::random_samples()` must not panic on empty storage
- `fetch_keys`/`fetch_entries` must handle cursor past-end gracefully (return empty items, finished = true)
- No phase/spec/bug references in code comments
- All new public items must have doc comments

## Assumptions

- `rand = "0.9"` is acceptable as a dependency (standard crate, no heavy transitive deps)
- Reservoir sampling is acceptable for `random_samples()` in Phase 3; a more efficient approach can be optimized later
- Cursor-based iteration using offset into a snapshot Vec is acceptable for Phase 3 partition sizes
- `NullDataStore` is a unit struct (no fields) -- it has no state to manage
- `HashMapStorage` does not need `Default` derive (use `HashMapStorage::new()` instead) but can optionally derive it

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `rand` to Cargo.toml. Create `storage/engines/mod.rs` + `storage/engines/hashmap.rs` (HashMapStorage implementation). Update `storage/mod.rs` with `pub mod engines;` and `pub mod datastores;` and re-exports. Add HashMapStorage unit tests. | -- | ~20% |
| G2 | 1 | Create `storage/datastores/mod.rs` + `storage/datastores/null.rs` (NullDataStore implementation). Add NullDataStore unit tests. | -- | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |

**Total workers needed:** 2 (max in Wave 1)

## Audit History

### Audit v1 (2026-02-21)
**Status:** NEEDS_REVISION

**Context Estimate:** ~33% total

**Critical:**
1. **Parallel write conflict on `storage/mod.rs`:** G1 and G2 both modify `packages/server-rust/src/storage/mod.rs` but are in the same wave (parallel). Worker G1 would add `pub mod engines;` and worker G2 would add `pub mod datastores;` -- the second writer overwrites the first. Fix: assign all `storage/mod.rs` modifications to G1 (which already modifies `Cargo.toml`), and remove the `storage/mod.rs` modification from G2. Updated task group descriptions above reflect this fix -- apply the same change to the spec body.

**Recommendations:**
2. [Compliance] File count (6) exceeds Language Profile limit (5) by 1. The two `mod.rs` re-export files are trivial (2-3 lines each) and the `Cargo.toml` change is 1 line, so the spirit of the limit is respected. If this pattern recurs, consider counting only substantive files. No action required, but note for future specs.
3. The `rand` crate version `0.8` is the current stable series but `0.9` has been released (as of early 2026). Consider whether `rand = "0.9"` would be more appropriate to avoid a future migration. This is low priority since `0.8` is widely used and stable.

### Response v1 (2026-02-21)
**Applied:** All 3 items (1 critical + 2 recommendations)

**Changes:**
1. [x] **Parallel write conflict on `storage/mod.rs`** -- Updated "Files to Modify" section to annotate that `storage/mod.rs` is modified by G1 only and G2 does not touch it. Updated "Sub-module re-exports" section to note `storage/mod.rs` update is done by G1. Task group table was already corrected during audit.
2. [x] **File count compliance** -- Acknowledged. No spec change needed; noted for future specs.
3. [x] **rand version** -- Updated `rand = "0.8"` to `rand = "0.9"` in all three occurrences: "Files to Modify" section, "Dependencies" section, and "Assumptions" section.

### Audit v2 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~30% total

**Dimensions evaluated:**
- Clarity: All methods specified with exact behavior and return values
- Completeness: Files, dependencies, re-exports, and edge cases all covered
- Testability: 5 acceptance criteria with concrete, measurable assertions
- Scope: Properly bounded -- two implementations, no scope creep
- Feasibility: DashMap + reservoir sampling is sound and straightforward
- Architecture fit: Matches existing storage module patterns from SPEC-058a
- Non-duplication: No existing implementations; reuses shared types
- Cognitive load: Simple wrappers around DashMap and no-op returns
- Strategic fit: Aligned with Phase 3 Rust server migration goals
- Project compliance: Honors PROJECT.md decisions and constraints

**Rust auditor checklist:**
- [x] No `f64` for integer-semantic fields (`estimated_cost` returns `u64`, cursor offset is `u64`)
- [x] No `r#type: String` (not applicable -- not message structs)
- [x] `Default` derive (not applicable -- no payload structs with 2+ optional fields)
- [x] Enums for known value sets (not applicable)
- [x] Wire compatibility (not applicable -- internal storage, not serialized)
- [x] `#[serde(rename_all)]` (not applicable)
- [x] Option skip/default (not applicable)

**Language profile:** File count (6) exceeds limit (5) by 1; previously acknowledged as acceptable (trivial re-export files). No trait-first violation -- traits exist in completed SPEC-058a.

**Parallel write conflict (v1 critical):** Verified resolved. `storage/mod.rs` is now explicitly assigned to G1 only, with G2 creating only its own files.

**Recommendations:**
1. The spec references `rand::thread_rng()` for reservoir sampling, but `rand 0.9` renamed this to `rand::rng()`. The implementer should use the `rand 0.9` API (`rand::rng()` or `rand::Rng` trait on `&mut impl Rng`). This is a non-blocking implementation detail since the spec describes behavior, not compilable code.

**Comment:** Well-structured spec with clear separation of concerns. All v1 critical issues properly resolved. Both implementations are straightforward with comprehensive acceptance criteria. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-21
**Mode:** orchestrated
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |

### Files Created
- `packages/server-rust/src/storage/engines/mod.rs` — Re-export module for HashMapStorage
- `packages/server-rust/src/storage/engines/hashmap.rs` — DashMap-backed StorageEngine implementation with cursor pagination, reservoir sampling, and 12 unit tests
- `packages/server-rust/src/storage/datastores/mod.rs` — Re-export module for NullDataStore
- `packages/server-rust/src/storage/datastores/null.rs` — No-op MapDataStore implementation with 13 unit tests

### Files Modified
- `packages/server-rust/Cargo.toml` — Added `rand = "0.9"` dependency
- `packages/server-rust/src/storage/mod.rs` — Added `pub mod engines;`, `pub mod datastores;` and re-exports

### Acceptance Criteria Status
- [x] `cargo build -p topgun-server` compiles with zero errors and zero warnings
- [x] `cargo clippy -p topgun-server -- -D warnings` passes clean
- [x] HashMapStorage unit tests: put/get/remove, contains_key, len/is_empty, clear, fetch_keys pagination, fetch_entries pagination, snapshot_iter, random_samples, estimated_cost
- [x] NullDataStore unit tests: Ok returns, load/load_all, is_null, is_loadable, pending_operation_count, soft_flush
- [x] Re-exports accessible via `crate::storage::{HashMapStorage, NullDataStore}`

### Deviations
1. [Rule 3 - Compilation] Fixed clippy `cast_possible_truncation` on cursor u64->usize casts — added `#[allow]` with WHY-comment
2. [Rule 1 - Bug] Fixed `Timestamp.node_id` type in test helper — was `0` (integer), should be `String::new()`

### Notes
- 99 total tests pass (25 new: 12 HashMapStorage + 13 NullDataStore)
- rand 0.9 API used correctly (`rand::rng()`, `random_range`) per audit recommendation
- Both workers ran in parallel; G1 handled all shared file modifications as specified

---

## Review History

### Review v1 (2026-02-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC1: `cargo build -p topgun-server` compiles with zero errors and zero warnings -- verified
- [x] AC2: `cargo clippy -p topgun-server -- -D warnings` passes clean -- verified
- [x] AC3: HashMapStorage unit tests (10 tests) -- all pass: put/get/remove round-trip, contains_key, len/is_empty, clear, fetch_keys pagination, fetch_entries pagination, snapshot_iter, random_samples (count + empty), estimated_cost
- [x] AC4: NullDataStore unit tests (13 tests) -- all pass: add, add_backup, remove, remove_backup, remove_all, load (None), load_all (empty), is_null (true), is_loadable (true), pending_operation_count (0), soft_flush (0), hard_flush, flush_key
- [x] AC5: Re-exports work -- `storage::HashMapStorage` and `storage::NullDataStore` accessible via `pub use engines::*` and `pub use datastores::*` in `storage/mod.rs`
- [x] Constraint: SPEC-058a traits not modified -- confirmed via git log (only original creation commits)
- [x] Constraint: No PostgreSQL/sqlx dependencies added -- confirmed via Cargo.toml grep
- [x] Constraint: No `f64` used for integer-semantic fields -- cursor offset is `u64`, estimated_cost is `u64`
- [x] Constraint: `random_samples` does not panic on empty storage -- early return for `sample_count == 0`, empty DashMap iteration for non-zero count
- [x] Constraint: Cursor past-end handled gracefully -- `skip(offset).take(size)` yields empty when offset >= total, `finished` set correctly
- [x] Constraint: No phase/spec/bug references in code comments -- verified
- [x] Constraint: All public items have doc comments -- `HashMapStorage`, `HashMapStorage::new()`, `NullDataStore` all documented
- [x] `rand = "0.9"` added to Cargo.toml -- correct
- [x] `rand::rng()` and `random_range` used (correct rand 0.9 API, not deprecated `thread_rng`)
- [x] Reservoir sampling algorithm is textbook correct (Vitter's Algorithm R)
- [x] Cursor encoding uses little-endian u64 as specified
- [x] `Default` impl provided for `HashMapStorage` (bonus, not required but good practice with `#[must_use]` on `new()`)
- [x] `#[allow(clippy::cast_possible_truncation)]` on cursor u64->usize casts has appropriate WHY-comment
- [x] Helper functions `decode_cursor_offset` / `encode_cursor_offset` are private (not polluting public API)
- [x] NullDataStore correctly implements all 14 MapDataStore trait methods
- [x] 97 total tests pass (all pre-existing + 23 new)

**Minor:**
1. Execution summary says "22 new" tests but the actual count is 23 (10 HashMapStorage + 13 NullDataStore; the `flush_key_returns_ok` test was likely miscounted). This is a documentation-only discrepancy.
2. No explicit unit test for cursor past-end edge case (offset beyond storage size). The behavior is correct by construction (`skip` past end yields empty, `finished = true`), but an explicit test would document the contract.

**Quality Assessment:**
- Code is clean, idiomatic Rust with appropriate DashMap delegation
- Module structure follows established patterns from the codebase
- Tests are well-structured with descriptive names
- No unnecessary `.clone()` calls beyond what DashMap's Ref requires
- Error handling uses `?` operator where applicable (N/A -- StorageEngine is synchronous, MapDataStore uses `anyhow::Result`)
- No `unsafe` code
- No hardcoded secrets or security concerns
- Cognitive load is low -- straightforward trait implementations with thin delegation
- No code duplication; cursor encoding extracted into shared helpers

**Implementation Reality Check:** No strategic concerns. Implementation complexity matches spec expectations. Both structs are thin wrappers as designed.

**Summary:** Clean, correct implementation that meets all 5 acceptance criteria and all constraints. Both `HashMapStorage` and `NullDataStore` faithfully implement their respective traits with comprehensive test coverage. The reservoir sampling algorithm is correct, cursor pagination works as specified, and the code integrates naturally with the existing storage module structure. Two minor documentation/test-coverage items noted but neither affects correctness.

### Fix Response v1 (2026-02-21)
**Applied:** All 2 minor items from Review v1

**Fixes:**
1. [x] Execution summary test count discrepancy — Updated "22 new" to "25 new" (12 HashMapStorage + 13 NullDataStore), corrected file descriptions (10→12 for hashmap.rs, 12→13 for null.rs), total 99.
2. [x] Cursor past-end edge case tests — Added `fetch_keys_cursor_past_end` and `fetch_entries_cursor_past_end` tests verifying empty items + finished=true when cursor offset exceeds storage size.
   - Commit: 06780bb

---

### Review v2 (2026-02-21 15:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Focus:** Re-review after Fix Response v1 addressed 2 minor items from Review v1.

**Fix Verification:**

**Fix 1 — Test count documentation:**
- Execution summary now reads "25 new: 12 HashMapStorage + 13 NullDataStore" -- correct
- `hashmap.rs` has 12 `#[test]` functions -- confirmed by inspection
- `null.rs` has 13 test functions (3 `#[test]` + 10 `#[tokio::test]`) -- confirmed by inspection
- Total 99 tests pass -- confirmed by `cargo test` output
- Fix correctly applied.

**Fix 2 — Cursor past-end edge case tests:**
- `fetch_keys_cursor_past_end` present at `hashmap.rs:347` -- confirmed
- `fetch_entries_cursor_past_end` present at `hashmap.rs:364` -- confirmed
- Both tests construct a cursor with `state = encode_cursor_offset(100)` against a 2-entry storage, assert `items.is_empty()` and `next_cursor.finished == true` -- logically correct
- Commit `06780bb` confirmed in git log
- Fix correctly applied.

**Build / Clippy / Test Results:**
- `cargo build -p topgun-server`: exit 0, zero errors, zero warnings -- PASS
- `cargo clippy -p topgun-server -- -D warnings`: exit 0, zero warnings -- PASS
- `cargo test -p topgun-server`: 99 passed, 0 failed, 0 ignored -- PASS

**No new issues introduced by the fixes.** The two added tests reuse the existing `encode_cursor_offset` helper (already private, already tested), construct `IterationCursor` directly (no new public API), and follow the same pattern as the existing pagination tests. No regressions.

**Passed:**
- [x] AC1: `cargo build -p topgun-server` -- zero errors, zero warnings
- [x] AC2: `cargo clippy -p topgun-server -- -D warnings` -- clean
- [x] AC3: HashMapStorage unit tests (12 tests) -- all 12 pass, including the 2 new past-end tests
- [x] AC4: NullDataStore unit tests (13 tests) -- all 13 pass
- [x] AC5: Re-exports via `crate::storage::HashMapStorage` and `crate::storage::NullDataStore` -- confirmed in `storage/mod.rs`
- [x] Constraint: No phase/spec/bug references in new test code -- confirmed
- [x] All fixes from Fix Response v1 correctly applied and verified

**Summary:** Both fixes from Review v1 are correctly applied. The cursor past-end tests are well-written and exercise exactly the specified edge case. All 99 tests pass, build and clippy are clean. No new issues introduced. Implementation is complete and ready to finalize.

---

## Completion

**Completed:** 2026-02-21
**Total Commits:** 5 (4 implementation + 1 fix)
**Audit Cycles:** 2
**Review Cycles:** 2
