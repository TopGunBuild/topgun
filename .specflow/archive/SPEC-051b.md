---
id: SPEC-051b
parent: SPEC-051
type: feature
status: done
priority: P0
complexity: small
depends_on: [SPEC-051a]
created: 2026-02-13
phase: "Phase 2: Rust Core"
---

# LWWMap Implementation

## Context

This is the second of three sub-specifications split from SPEC-051 (Port Core CRDTs to Rust). SPEC-051a provides the foundation: `HLC`, `Timestamp`, `ClockSource`, `MerkleTree`, FNV-1a hash, `Value` enum, and shared record types (`LWWRecord<V>`). This spec implements the `LWWMap<V>` (Last-Write-Wins Map) on top of that foundation.

`LWWMap` is one of the two core CRDT data structures in TopGun. It provides conflict-free convergence by always keeping the entry with the highest HLC timestamp. It integrates with `MerkleTree` for efficient delta synchronization.

This spec can execute in parallel with SPEC-051c (ORMap), as they share no dependencies beyond the foundation.

### Reference

- **TypeScript behavioral spec:** `packages/core/src/LWWMap.ts` (~220 lines)
- **TypeScript tests:** `packages/core/src/__tests__/LWWMap.test.ts`
- **Foundation types (from SPEC-051a):** `Timestamp`, `HLC`, `ClockSource`, `MerkleTree`, `LWWRecord<V>`, `Value`

## Goal Analysis

**Goal Statement:** The Rust `topgun-core` crate provides a production-ready `LWWMap<V>` implementation that is behaviorally equivalent to the TypeScript `LWWMap`, with verified commutativity and idempotence via property-based tests.

**Observable Truths:**

1. `LWWMap::set()` stores a value with an HLC timestamp; `LWWMap::get()` retrieves it (filtering tombstones and expired TTL records)
2. `LWWMap::merge()` resolves conflicts identically to TS: higher timestamp wins (millis > counter > node_id)
3. `LWWMap::merge()` is commutative: merging R1 then R2 produces the same state as merging R2 then R1
4. `LWWMap::merge()` is idempotent: merging the same record twice does not change state
5. `LWWMap::remove()` creates a tombstone; `prune()` removes tombstones older than threshold
6. MerkleTree is updated on every `set()`, `remove()`, and `merge()`
7. `LWWRecord<Value>` round-trips through `rmp_serde` without data loss

**Required Artifacts:**

| Truth | File(s) |
|-------|---------|
| 1-6 | `lww_map.rs` (implementation + tests) |
| 7 | `lww_map.rs` (round-trip test) |
| all | `lib.rs` (re-exports) |

## Task

Implement `LWWMap<V>` in `packages/core-rust/src/lww_map.rs`. The map uses `String` keys, stores `LWWRecord<V>` values, integrates with `HLC` for timestamp generation and `MerkleTree` for sync hashing. Add comprehensive unit tests mirroring TS test vectors and property-based tests with `proptest` for commutativity, idempotence, and convergence.

## Requirements

### Files to Create

1. **`packages/core-rust/src/lww_map.rs`** -- Last-Write-Wins Map
   - `LWWMap<V>` struct (keys are `String`) with:
     - `new(hlc: HLC) -> Self` -- constructor taking ownership of an HLC instance
     - `set(&mut self, key: impl Into<String>, value: V, ttl_ms: Option<u64>) -> LWWRecord<V>` -- stores value with new HLC timestamp
     - `get(&self, key: &str) -> Option<&V>` -- returns value if exists, not tombstoned, and not TTL-expired
     - `get_record(&self, key: &str) -> Option<&LWWRecord<V>>` -- returns full record including timestamp
     - `remove(&mut self, key: &str) -> LWWRecord<V>` -- creates tombstone (value: None) with new timestamp. **Note:** Always creates and stores a tombstone even if the key does not exist, matching TS behavior where tombstones are created regardless of prior existence.
     - `merge(&mut self, key: impl Into<String>, remote_record: LWWRecord<V>) -> bool` -- merges remote record, returns true if local state changed. **Note:** Always calls `let _ = self.hlc.update(&remote_record.timestamp);` to maintain causality, silently ignoring errors (matching TS behavior where `hlc.update()` is void).
     - `prune(&mut self, older_than: &Timestamp) -> Vec<String>` -- removes tombstones older than threshold, returns pruned keys
     - `clear(&mut self)` -- removes all data and resets MerkleTree
     - `entries(&self) -> impl Iterator<Item = (&String, &V)>` -- iterates non-tombstone, non-expired entries
     - `all_keys(&self) -> impl Iterator<Item = &String>` -- iterates all keys including tombstones
     - `size(&self) -> usize` -- number of entries (including tombstones)
     - `merkle_tree(&self) -> &MerkleTree` -- read-only access to internal MerkleTree
   - Internal storage: `HashMap<String, LWWRecord<V>>`
   - Merge logic: accept remote if no local record OR `remote.timestamp > local.timestamp` (using `Timestamp::Ord`)
   - Tombstone: `LWWRecord { value: None, timestamp, ttl_ms: None }`
   - TTL: record expired when `record.timestamp.millis + ttl_ms < clock_source.now()`
   - MerkleTree update: on every `set()`, `remove()`, `merge()` that changes state, compute item hash as `fnv1a_hash("{key}:{ts.millis}:{ts.counter}:{ts.node_id}")` and call `merkle_tree.update(key, item_hash)`
   - MerkleTree remove: on `prune()`, call `merkle_tree.remove(key)` for each pruned key
   - Trait bounds on `V`: `Clone + Serialize + DeserializeOwned + PartialEq`

### Files to Modify

2. **`packages/core-rust/src/lib.rs`** -- Add LWWMap module and re-exports
   - Add `pub mod lww_map;`
   - Add re-export: `LWWMap`

### Behavioral Equivalence Requirements

The following TS test vectors MUST pass in Rust:

**LWWMap:**
- `set()` + `get()` basic CRUD: set a value, get returns it
- `remove()` creates tombstone: after remove, `get()` returns `None`
- Conflict resolution: higher millis wins; equal millis -> higher counter wins; equal both -> higher nodeId wins
- Merge is commutative: `merge(A, B)` produces same state as `merge(B, A)`
- Merge is idempotent: `merge(A, A)` does not change state
- TTL: record expired when `timestamp.millis + ttl_ms < clock.now()`
- `prune()`: only removes tombstones older than threshold; non-tombstone records are untouched
- `entries()`: skips tombstones and expired records
- MerkleTree updates on every `set()`, `remove()`, `merge()`
- MerkleTree root hash changes after set/remove, returns to 0 after clear

## Acceptance Criteria

1. **AC-1:** `cargo build -p topgun-core` succeeds with zero errors and zero warnings (`cargo clippy -p topgun-core` clean).
2. **AC-2:** `cargo test -p topgun-core` passes all tests (existing + new LWWMap tests).
3. **AC-3:** `LWWMap::merge()` is commutative: for any two records R1, R2 and key K, merging R1 then R2 produces same state as merging R2 then R1. Verified by proptest with 100+ cases.
4. **AC-4:** `LWWMap::merge()` is idempotent: merging the same record twice does not change state. Verified by proptest.
5. **AC-5:** `LWWRecord<Value>` round-trips through `rmp_serde::to_vec()` / `rmp_serde::from_slice()` without data loss. Verified by serialization round-trip test.
6. **AC-6:** `cargo doc -p topgun-core --no-deps` produces zero warnings. All public `LWWMap` types and functions have doc comments.

## Constraints

- Do NOT use external CRDT crates (`yrs`, `crdts`, `automerge`). Custom implementation only.
- Do NOT add `tokio` as a dependency. `LWWMap` is synchronous.
- Do NOT add change listener/callback functionality. The TS `onChange()` pattern is UI-specific and not needed server-side.
- `LWWMap` keys are `String` (not generic `K`) to simplify MerkleTree integration, matching TS usage patterns.

## Assumptions

1. **Generic value type `V`.** `LWWMap` is generic over `V` with bounds `Clone + Serialize + DeserializeOwned + PartialEq`. The `Value` enum is one concrete instantiation.
2. **No IndexedLWWMap.** Query indexes are a separate concern for a future spec.
3. **No ConflictResolver.** Custom merge strategies are not needed; default LWW behavior is sufficient.
4. **TTL checking uses `ClockSource`** from the HLC instance, matching the TS pattern where `hlc.getClockSource().now()` is used for expiry checks.
5. **HLC is owned by LWWMap.** The `LWWMap` takes ownership of the `HLC` instance (or an `Arc<HLC>` if sharing is needed). For proptest, a deterministic `ClockSource` can be injected into the HLC before constructing the map.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `LWWMap<V>` struct definition, constructor, `set()`, `get()`, `get_record()`, `remove()`, `merge()`, `prune()`, `clear()`, `entries()`, `all_keys()`, `size()`, `merkle_tree()`. Internal `update_merkle()` helper. Unit tests: basic CRUD, tombstone behavior, TTL expiry, conflict resolution (higher millis/counter/nodeId), prune only removes tombstones, MerkleTree integration (hash changes on mutation). | -- | ~28% |
| G2 | 2 | Proptest strategies: `Arbitrary` impl for `Timestamp` and `LWWRecord<Value>`. Proptest cases: commutativity (100+ cases), idempotence (100+ cases), convergence (multiple records merged in different orders produce same state). | G1 | ~12% |
| G3 | 2 | Update `lib.rs` with `pub mod lww_map;` and `LWWMap` re-export. MsgPack round-trip test for `LWWRecord<Value>`. Verify `cargo clippy` and `cargo doc` clean. | G1 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-13 18:30)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~45% | <=50% | OK |
| Largest task group (G1) | ~28% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Per-Group Breakdown (auditor re-estimate):**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | LWWMap impl + unit tests | ~28% | 28% |
| G2 | 2 | Proptest strategies + property tests | ~12% | 40% |
| G3 | 2 | lib.rs re-exports + round-trip test | ~5% | 45% |

**Goal Analysis Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 has artifacts | OK | lww_map.rs |
| Truth 2 has artifacts | OK | lww_map.rs |
| Truth 3 has artifacts | OK | lww_map.rs (proptest) |
| Truth 4 has artifacts | OK | lww_map.rs (proptest) |
| Truth 5 has artifacts | OK | lww_map.rs |
| Truth 6 has artifacts | OK | lww_map.rs |
| Truth 7 has artifacts | OK | lww_map.rs (round-trip test) |
| All artifacts have purpose | OK | No orphans |
| Wiring: LWWMap -> HLC | OK | Ownership specified |
| Wiring: LWWMap -> MerkleTree | OK | Internal field, update on mutation |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | HLC ownership model (not shared via Arc) | Low: proptest needs deterministic clock, achievable via ClockSource injection |
| A2 | No IndexedLWWMap needed | None: deferred to future spec |
| A3 | String keys sufficient (not generic K) | None: matches TS usage, simplifies MerkleTree integration |
| A4 | V: Clone + Serialize + DeserializeOwned + PartialEq sufficient | Low: could need Debug for error messages, but not required |

**Project Compliance:** OK -- Honors PROJECT.md decisions (MsgPack, custom CRDTs, no tokio, Rust migration Phase 2)

**Language Profile:** OK -- 2 files (within 5-file limit). No new traits defined, so trait-first ordering not applicable. G1 includes implementation which is appropriate here.

**Strategic Fit:** OK -- Aligned with project goals. LWWMap is a core CRDT required for Rust migration Phase 2.

**Recommendations:**

1. The `merge()` method calls `hlc.update(&remote_record.timestamp)` which returns `Result<(), String>` in the Rust HLC. The spec does not specify error handling. Recommend: silently ignore the error (matching TS behavior where `hlc.update()` is void), or use `let _ = self.hlc.update(...)`. The implementer should be aware this can fail in strict mode.

2. The spec's G1 context estimate of ~50% appears overestimated. Based on file complexity analysis (single ~300-line Rust file with tests, reading 3 existing modules), ~28% is more realistic. The overestimation is conservative and not harmful, but may unnecessarily trigger decomposition concerns.

3. Consider adding a note that `remove()` on a non-existent key should still create and store a tombstone (matching TS behavior where a tombstone is created regardless of prior existence). The current wording implies this but does not state it explicitly.

### Response v1 (2026-02-13 18:45)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [✓] Recommendation 1 (hlc.update error handling) — Added explicit note in `merge()` method signature: "Always calls `let _ = self.hlc.update(&remote_record.timestamp);` to maintain causality, silently ignoring errors (matching TS behavior where `hlc.update()` is void)."

2. [✓] Recommendation 2 (G1 context estimate) — Fixed G1 context estimate from ~50% to ~28% in Implementation Tasks table, and updated G2 from ~30% to ~12%, G3 from ~10% to ~5%. Total remains ~45%.

3. [✓] Recommendation 3 (remove() tombstone behavior) — Added explicit note in `remove()` method signature: "Always creates and stores a tombstone even if the key does not exist, matching TS behavior where tombstones are created regardless of prior existence."

### Audit v2 (2026-02-13 19:15)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Comment:** Fresh-eyes re-audit after revision. All 3 recommendations from Audit v1 were properly addressed in the spec body. Verified against existing codebase: HLC API (`now()`, `update()`, `clock_source()`), MerkleTree API (`update(key, item_hash)`, `remove(key)`, `get_root_hash()`), LWWRecord struct, Timestamp Ord implementation, and fnv1a_hash function all match the spec's usage patterns. The TS behavioral reference (`packages/core/src/LWWMap.ts`) confirms merge semantics (strict greater-than, not >=), tombstone creation on remove (regardless of prior existence), and TTL expiry logic. proptest is already in dev-dependencies. No critical issues found. Spec is complete, clear, and immediately implementable.

## Execution Summary

**Executed:** 2026-02-14
**Mode:** orchestrated (sequential fallback -- no subagent spawning available)
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |

### Files Created
- `packages/core-rust/src/lww_map.rs` -- LWWMap implementation, 39 unit tests, 6 proptests, 3 MsgPack round-trip tests

### Files Modified
- `packages/core-rust/src/lib.rs` -- Added `pub mod lww_map;`, `LWWMap` re-export, crate doc update, re-export test

### Test Summary
- 132 unit/proptest tests + 5 doc-tests = 137 total, all passing
- 36 LWWMap unit tests (CRUD, tombstone, TTL, conflict resolution, prune, MerkleTree integration)
- 3 MsgPack round-trip tests (value, tombstone, all variants)
- 6 proptest property tests (commutativity, idempotence, convergence, round-trip, winner-has-highest, multi-key)

### Acceptance Criteria Status
- [x] AC-1: `cargo build -p topgun-core` zero errors; `cargo clippy` clean
- [x] AC-2: `cargo test -p topgun-core` passes all 137 tests
- [x] AC-3: Commutativity verified by proptest (256+ cases)
- [x] AC-4: Idempotence verified by proptest (256+ cases)
- [x] AC-5: `LWWRecord<Value>` MsgPack round-trip verified (3 explicit + proptest)
- [x] AC-6: `cargo doc -p topgun-core --no-deps` zero warnings; all public types documented

### Deviations
None.

---

## Review History

### Review v1 (2026-02-14 14:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Passed:**

- [x] **AC-1: Build and Clippy** -- `cargo build -p topgun-core` succeeds with zero errors; `cargo clippy -p topgun-core -- -D warnings` clean (verified by reviewer)
- [x] **AC-2: All Tests Pass** -- `cargo test -p topgun-core` passes 132 unit/proptest tests + 5 doc-tests = 137 total (verified by reviewer)
- [x] **AC-3: Commutativity** -- `merge_is_commutative` proptest covers all record types including tombstones, using `arb_lww_record_maybe_tombstone()` strategy with 256+ cases
- [x] **AC-4: Idempotence** -- `merge_is_idempotent` proptest verifies both record equality and MerkleTree hash stability after double merge
- [x] **AC-5: MsgPack Round-trip** -- 3 explicit tests (value, tombstone, all Value variants) + proptest `lww_record_msgpack_roundtrip_proptest` covering random records
- [x] **AC-6: Documentation** -- `cargo doc -p topgun-core --no-deps` produces zero warnings; all public types and functions have doc comments including a working doc-test example on `LWWMap`
- [x] **File Created:** `packages/core-rust/src/lww_map.rs` exists (1170 lines, implementation + comprehensive tests)
- [x] **File Modified:** `packages/core-rust/src/lib.rs` includes `pub mod lww_map;` and `pub use lww_map::LWWMap;`
- [x] **No Files to Delete** -- spec had no deletions
- [x] **Constraint: No external CRDT crates** -- No `yrs`, `crdts`, or `automerge` in dependencies or imports
- [x] **Constraint: No tokio** -- Not present in dependencies
- [x] **Constraint: No onChange callbacks** -- Correctly omitted per spec
- [x] **Constraint: String keys** -- `LWWMap<V>` uses `String` keys, not generic `K`
- [x] **Behavioral Equivalence: set/get CRUD** -- Tests `set_and_get_basic`, `get_nonexistent_key_returns_none`, `set_overwrites_existing_value`
- [x] **Behavioral Equivalence: Tombstones** -- Tests `remove_creates_tombstone`, `remove_nonexistent_key_creates_tombstone`, `size_includes_tombstones`
- [x] **Behavioral Equivalence: Conflict Resolution** -- Tests `conflict_higher_millis_wins`, `conflict_higher_counter_wins`, `conflict_higher_node_id_wins` covering all three tiebreaker levels
- [x] **Behavioral Equivalence: TTL** -- Tests `ttl_not_expired_returns_value`, `ttl_expired_returns_none`, `ttl_boundary_not_expired`, `ttl_none_never_expires` covering expiry semantics including boundary condition
- [x] **Behavioral Equivalence: Prune** -- Tests `prune_removes_old_tombstones`, `prune_does_not_remove_recent_tombstones`, `prune_does_not_remove_non_tombstones`, `prune_returns_empty_on_empty_map`
- [x] **Behavioral Equivalence: entries()** -- Tests `entries_skips_tombstones`, `entries_skips_expired_ttl`
- [x] **Behavioral Equivalence: MerkleTree integration** -- Tests for hash updates on set/remove/merge, prune removes entry, clear resets to 0, deterministic hashing
- [x] **Merge semantics match TS** -- Uses strict greater-than (`remote_record.timestamp > local.timestamp`), matching TS `HLC.compare() > 0`
- [x] **HLC causality on merge** -- `let _ = self.hlc.update(&remote_record.timestamp);` called before merge logic, silently ignoring errors per spec
- [x] **MerkleTree hash formula** -- `fnv1a_hash("{key}:{millis}:{counter}:{node_id}")` matches spec exactly
- [x] **Trait bounds** -- `V: Clone + Serialize + DeserializeOwned + PartialEq` as specified
- [x] **No `unsafe` blocks** -- Zero unsafe code in production or test
- [x] **No `.unwrap()` in production code** -- All `unwrap()`/`expect()` calls confined to `#[cfg(test)]` modules
- [x] **No unnecessary `.clone()` calls** -- Cloning is appropriate where required (returning records, proptest data)
- [x] **`#[must_use]` annotations** -- Applied to all pure accessor methods (`new`, `get`, `get_record`, `size`, `merkle_tree`)
- [x] **No hardcoded secrets** -- Clean
- [x] **No spec/phase references in code comments** -- WHY-comments only
- [x] **Proptest convergence** -- `merge_convergence_three_records` tests all 6 permutations of 3 records; `multi_key_convergence` tests forward/reverse order across multiple keys
- [x] **`FixedClock` test infrastructure** -- Deterministic clock properly duplicated in both `tests` and `proptests` modules (acceptable since they are separate `#[cfg(test)]` modules in the same file)
- [x] **Language Profile compliance** -- 2 files (within 5-file limit); no new traits defined so trait-first not applicable
- [x] **Architecture alignment** -- Follows existing patterns from SPEC-051a (same module structure, same test infrastructure patterns, same doc comment style)

**Summary:** Implementation is clean, complete, and fully compliant with the specification. All 6 acceptance criteria verified independently by the reviewer. The code is well-structured with clear separation between the 244-line production implementation and ~926 lines of comprehensive tests. Behavioral equivalence with the TypeScript `LWWMap` has been verified by cross-referencing the TS source. The proptest suite provides strong CRDT correctness guarantees beyond what unit tests alone could offer. No critical, major, or minor issues found.

---

## Completion

**Completed:** 2026-02-14
**Total Commits:** 2
**Audit Cycles:** 2
**Review Cycles:** 1

---
*Parent: SPEC-051. Depends on: SPEC-051a. Sibling: SPEC-051c (ORMap). Source: TODO-061 (Phase 2). TS behavioral specification: `packages/core/src/LWWMap.ts`.*
