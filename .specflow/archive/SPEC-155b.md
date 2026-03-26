---
id: SPEC-155b
type: feature
status: done
priority: P2
complexity: small
created: 2026-03-25
parent: SPEC-155
depends_on: [SPEC-155a]
---

# Index Registry and Mutation Observer

## Context

With the three index types (HashIndex, NavigableIndex, InvertedIndex) and supporting types implemented in SPEC-155a, this spec adds the management layer: an `IndexRegistry` that holds all indexes for a given map, and an `IndexMutationObserver` that integrates with the existing `MutationObserver` pipeline to keep indexes in sync with record mutations.

The `MutationObserver` trait is defined in `packages/server-rust/src/storage/mutation_observer.rs`. The `ObserverFactory` trait is defined in `packages/server-rust/src/storage/factory.rs`. Both are already used by the `MerkleMutationObserver` for Merkle tree sync. This spec follows the same pattern to wire index updates into the observer chain.

**Existing codebase context:**
- `MutationObserver` trait methods: `on_put`, `on_update`, `on_remove`, `on_evict`, `on_load`, `on_replication_put`, `on_clear`, `on_reset`, `on_destroy`
- `ObserverFactory::create_observer(map_name, partition_id) -> Option<Arc<dyn MutationObserver>>`
- `CompositeMutationObserver` fans out to multiple observers
- `RecordStoreFactory` holds `Vec<Arc<dyn ObserverFactory>>` and assembles composite observers at store creation

## Task

Create 2 files in `packages/server-rust/src/service/domain/index/`: `registry.rs` (IndexRegistry) and `mutation_observer.rs` (IndexMutationObserver + IndexObserverFactory). Update `mod.rs` to re-export the new types.

## Requirements

### R1: IndexRegistry (`index/registry.rs`)

- Per-map registry: `DashMap<String, Arc<dyn Index>>` keyed by attribute name
- Methods: `add_hash_index(attr)`, `add_navigable_index(attr)`, `add_inverted_index(attr)` -- each creates the corresponding index type and inserts it
- `get_index(attr) -> Option<Arc<dyn Index>>`
- `get_best_index(predicate: &PredicateNode) -> Option<Arc<dyn Index>>` -- extracts the attribute and op from a leaf predicate and returns the index covering it using the following rules:
  - Returns `None` for compound predicates (`And`/`Or`/`Not`) -- compound predicate optimization belongs in the query optimizer (SPEC-155c)
  - `Eq`/`Neq` ops: return `Hash` index for the attribute if present, otherwise `None`
  - `Gt`/`Gte`/`Lt`/`Lte` ops: return `Navigable` index for the attribute if present, otherwise `None`
  - `Like` op: return `Inverted` index for the attribute if present (token-based partial match), otherwise `None`
  - `Regex` op: always return `None` (regex cannot be accelerated by these index types)
  - Returns `None` if no index of the required type exists for the attribute, even if an index of a different type is registered -- returning a mismatched index type would yield incorrect results (e.g. `HashIndex::lookup_range` returns an empty set)
- `indexes() -> Vec<Arc<dyn Index>>` -- returns all indexes (for iteration by mutation observer)
- `stats() -> Vec<IndexStats>` where `IndexStats { attribute: String, index_type: IndexType, entry_count: u64 }`
- Thread-safe: all operations are concurrent-safe via DashMap
- Unit tests: add/get indexes, stats accuracy, best index selection (including compound predicate returns None, type-mismatch returns None)

### R2: IndexMutationObserver (`index/mutation_observer.rs`)

- `IndexMutationObserver` holds `Arc<IndexRegistry>` for the map
- Implements `MutationObserver` trait:
  - `on_put`: for each index in registry, extract the attribute from the record value and call `index.insert(key, value)`
  - `on_update`: for each index, call `index.update(key, old_extracted, new_extracted)`
  - `on_remove`: for each index, call `index.remove(key, extracted)`
  - `on_clear` / `on_reset`: call `index.clear()` on all indexes
  - `on_evict`: call `index.remove(key, extracted)` on all indexes -- evicted records leave the store and must not remain in the index
  - `on_load`: call `index.insert(key, extracted)` on all indexes -- loaded records enter the store and must be indexed
  - `on_replication_put`: call `index.insert(key, extracted)` on all indexes when `populate_index` is true; no-op otherwise
  - `on_destroy`: call `index.clear()` on all indexes -- the store is being destroyed, all index state must be released
- `IndexObserverFactory` implements `ObserverFactory` trait:
  - Holds a registry of `DashMap<String, Arc<IndexRegistry>>` mapping map names to their registries
  - `create_observer(map_name, partition_id)` returns `Some(Arc<IndexMutationObserver>)` if the map has registered indexes, `None` otherwise
  - `register_map(map_name) -> Arc<IndexRegistry>` -- creates and returns an IndexRegistry for the map
- Note: record values need to be converted from `RecordValue` to `rmpv::Value` for attribute extraction -- extract the inner Value from `RecordValue::Lww` or use `value_to_rmpv` from `predicate.rs`. For `RecordValue::OrMap`, skip indexing (OR-Map records are multi-entry and not suitable for single-attribute indexes). For `RecordValue::OrTombstones`, skip indexing (no data to index).
- Unit tests: observer receives mutations and updates indexes, factory creates observers for registered maps

## Acceptance Criteria

1. **AC1:** Inserting a record triggers `IndexMutationObserver::on_put` which updates all indexes for that map
2. **AC2:** Removing a record removes its entries from all indexes (no stale index entries)
3. **AC3:** `IndexRegistry::stats()` returns correct entry counts and index types for all registered indexes
4. **AC4:** `on_clear()` empties all indexes; subsequent lookups return empty sets

## Validation Checklist

- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server -- index` -- all index tests pass
- Insert + remove 100 records through the mutation observer, verify `entry_count()` matches expected value (no leaks)
- Run `cargo test --release -p topgun-server` -- all existing tests still pass (no regressions)

## Constraints

- Do NOT auto-create indexes based on query patterns (that is adaptive indexing, a separate feature)
- Do NOT add wire protocol messages for index management in this spec
- Do NOT use `f64` for integer-semantic fields -- use `u64`/`u32`
- Index data structures are in-memory only
- Max 5 files modified/created in this spec (2 new + mod.rs update)

## Assumptions

- **Index creation is server-side only:** Indexes are created programmatically or via server config, not via client API
- **SPEC-155a is complete:** The Index trait, value types, AttributeExtractor, and all three index implementations are available

## Audit History

### Audit v1 (2026-03-26)
**Status:** NEEDS_REVISION

**Context Estimate:** ~18% total (2 new files ~7% each, mod.rs update ~1%, overhead ~3%)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`entry_count: u64` in `IndexStats`)
- [x] No `r#type: String` on message structs (no wire structs)
- [x] Enums used for known value sets (`IndexType` enum)
- [N/A] `Default` derived on payload structs (no payload structs)
- [N/A] Wire compatibility (no wire serialization)
- [N/A] `#[serde(rename_all = "camelCase")]` (no serde structs)
- [N/A] `#[serde(skip_serializing_if)]` (no Option serde fields)

Strategic fit: Aligned with project goals (indexing subsystem, SPEC-155 parent)
Project compliance: Honors PROJECT.md decisions (Rust, DashMap patterns, 5-file limit)
Language profile: Compliant with Rust profile (3 files, within 5-file limit)

**Critical:**
1. **Vague `on_evict`/`on_load`/`on_destroy` handling:** R2 says "appropriate handling" without specifying behavior. Must be explicit: `on_evict` should call `index.remove(key, extracted)` (evicted records are gone from storage). `on_load` should call `index.insert(key, extracted)` (loaded records enter the store). `on_destroy` should call `index.clear()` on all indexes. Without this, the implementer must guess and may introduce stale index entries or missing entries.
2. **`get_best_index` undefined for compound predicates:** `PredicateNode` can have `op: And|Or|Not` with `children` and no `attribute`. The spec only describes behavior for leaf predicates (Eq, Gt, etc.). Must specify: return `None` for compound predicates (And/Or/Not) -- compound predicate optimization belongs in the query optimizer (SPEC-155c).
3. **`get_best_index` unclear when index type doesn't match operation:** If attribute "age" has only a HashIndex but predicate is `Gt` (range op), should it return the HashIndex or `None`? Since `HashIndex::lookup_range` returns an empty set, returning it would be misleading. Must specify: return `None` if no index of the preferred type exists for the operation. Specifically: Eq/Neq -> Hash only, Gt/Gte/Lt/Lte -> Navigable only, Like -> Inverted only, Regex -> None (not indexable).

**Recommendations:**
4. `Like` and `Regex` predicate ops are not mapped to index types. Recommend specifying: `Like` returns Inverted index if available (for token-based partial match), `Regex` returns `None` (regex cannot be accelerated by these index types).
5. AC numbering has a gap (4, 5, 6, 10). If continuing from SPEC-155a numbering, consider noting this in the spec to avoid confusion. Alternatively, renumber locally as AC1-AC4 for this spec.

### Response v1 (2026-03-26)
**Applied:** All critical issues (1, 2, 3) and all recommendations (4, 5)

**Changes:**
1. [✓] Explicit `on_evict`/`on_load`/`on_destroy` handling — replaced vague "appropriate handling" bullet in R2 with four explicit bullets: `on_evict` calls `index.remove`, `on_load` calls `index.insert`, `on_replication_put` calls `index.insert` when `populate_index` is true, `on_destroy` calls `index.clear` on all indexes
2. [✓] `get_best_index` defined for compound predicates — added explicit rule to R1: returns `None` for `And`/`Or`/`Not` predicates; compound predicate optimization belongs in SPEC-155c
3. [✓] `get_best_index` type-match rule specified — added explicit op-to-index-type mapping in R1: `Eq`/`Neq` requires Hash, `Gt`/`Gte`/`Lt`/`Lte` requires Navigable, `Like` requires Inverted, `Regex` always returns `None`; returns `None` if no index of the required type exists even if a different type is registered
4. [✓] `Like` and `Regex` ops mapped — `Like` returns Inverted index if available, `Regex` always returns `None`; included in the unified type-match rule in R1
5. [✓] AC numbering gap resolved — renumbered AC4/AC5/AC6/AC10 to AC1/AC2/AC3/AC4 for local clarity

### Audit v2 (2026-03-26)
**Status:** APPROVED

**Context Estimate:** ~16% total (registry.rs ~5%, mutation_observer.rs ~5%, mod.rs ~1%, overhead ~5%)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`entry_count: u64` in `IndexStats`)
- [x] No `r#type: String` on message structs (no wire structs in this spec)
- [x] Enums used for known value sets (`IndexType` enum)
- [N/A] `Default` derived on payload structs (no payload structs)
- [N/A] Wire compatibility (no wire serialization)
- [N/A] `#[serde(rename_all = "camelCase")]` (no serde structs)
- [N/A] `#[serde(skip_serializing_if)]` (no Option serde fields)

Strategic fit: Aligned with project goals
Project compliance: Honors PROJECT.md decisions
Language profile: Compliant with Rust profile (3 files, within 5-file limit)

**v1 Resolution Verification:** All 3 critical issues and 2 recommendations from v1 have been addressed. R1 now has explicit compound predicate handling, complete op-to-index-type mapping, and type-mismatch rules. R2 now has explicit behavior for all 9 MutationObserver methods. AC numbering is clean.

**Recommendations:**
1. R2 conversion note mentions `RecordValue::Lww` but does not specify behavior for `OrMap` or `OrTombstones` variants. Recommend: for `OrMap`, skip indexing (OR-Map records are multi-entry and not suitable for single-attribute indexes); for `OrTombstones`, skip (no data to index). This is inferrable but explicit guidance prevents implementer hesitation.

### Response v2 (2026-03-26)
**Applied:** Recommendation 1

**Changes:**
1. [✓] `RecordValue` variant handling in R2 conversion note — expanded the note to explicitly specify behavior for all three `RecordValue` variants: `Lww` is indexed (extract inner Value), `OrMap` is skipped (multi-entry, not suitable for single-attribute indexes), `OrTombstones` is skipped (no data to index)

### Audit v3 (2026-03-26)
**Status:** APPROVED

**Context Estimate:** ~16% total (registry.rs ~5%, mutation_observer.rs ~5%, mod.rs ~1%, overhead ~5%)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`entry_count: u64` in `IndexStats`)
- [x] No `r#type: String` on message structs (no wire structs in this spec)
- [x] Enums used for known value sets (`IndexType` enum)
- [N/A] `Default` derived on payload structs (no payload structs)
- [N/A] Wire compatibility (no wire serialization)
- [N/A] `#[serde(rename_all = "camelCase")]` (no serde structs)
- [N/A] `#[serde(skip_serializing_if)]` (no Option serde fields)

Strategic fit: Aligned with project goals
Project compliance: Honors PROJECT.md decisions
Language profile: Compliant with Rust profile (3 files, within 5-file limit)

**v2 Resolution Verification:** Recommendation from v2 (RecordValue variant handling) has been incorporated. R2 now explicitly covers all three RecordValue variants (Lww, OrMap, OrTombstones).

**Comment:** Spec is well-structured and implementation-ready. All MutationObserver methods have explicit behavior, predicate-to-index-type mapping is complete with all edge cases covered (compound predicates, type mismatches, Regex, OrMap/OrTombstones). Requirements map cleanly to the existing ObserverFactory pattern in the codebase. No critical issues remain.

---

## Execution Summary

**Executed:** 2026-03-26
**Commits:** 3

### Files Created
- `packages/server-rust/src/service/domain/index/registry.rs` — IndexRegistry with DashMap-backed per-attribute index storage, add_hash/navigable/inverted_index methods, get_index, get_best_index (op-to-type mapping), indexes(), stats(), and unit tests
- `packages/server-rust/src/service/domain/index/mutation_observer.rs` — IndexMutationObserver implementing all 9 MutationObserver methods and IndexObserverFactory implementing ObserverFactory, with unit tests

### Files Modified
- `packages/server-rust/src/service/domain/index/mod.rs` — Added `#[derive(Debug, Clone, PartialEq, Eq)]` to `IndexType`; added `pub mod mutation_observer` and `pub mod registry`; re-exported `IndexMutationObserver`, `IndexObserverFactory`, `IndexRegistry`, `IndexStats`

### Acceptance Criteria Status
- [x] AC1: Inserting a record triggers `IndexMutationObserver::on_put` which updates all indexes for that map
- [x] AC2: Removing a record removes its entries from all indexes (no stale index entries); verified by `insert_remove_100_records_no_leaks` test
- [x] AC3: `IndexRegistry::stats()` returns correct entry counts and index types for all registered indexes
- [x] AC4: `on_clear()` empties all indexes; subsequent lookups return empty sets

### Deviations
1. [Rule 1 - Bug] `Value::Map` in topgun_core uses `BTreeMap` not `HashMap`; fixed test helper in mutation_observer.rs to use `std::collections::BTreeMap`
2. [Rule 1 - Bug] Clippy reported `match-same-arms` for compound/Regex return `None` branches; merged into single arm `And | Or | Not | Regex => return None`

### Notes
- `value_to_rmpv` is `pub(crate)` in `predicate.rs`; accessed as `crate::service::domain::predicate::value_to_rmpv` from the index module (same crate)
- Each index implementation holds its own `AttributeExtractor`; the observer passes the full rmpv record value to `index.insert/update/remove` and each index extracts its own attribute
- `IndexObserverFactory::register_map` is idempotent via DashMap `entry().or_insert_with()`; repeated calls return the same `Arc<IndexRegistry>`
- 651 total tests pass, no regressions

---

## Review History

### Review v1 (2026-03-26 14:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `on_put` test confirms all indexes are updated when a record is inserted
- [✓] AC2: `insert_remove_100_records_no_leaks` test verifies zero stale index entries after removing 100 records; `on_remove` and `on_evict` both delegate to `index.remove`
- [✓] AC3: `stats_accuracy` and `stats_entry_count_updates_after_insert` tests confirm correct attribute, index_type, and entry_count reporting
- [✓] AC4: `on_clear_empties_all_indexes` test confirms `entry_count()` returns 0 and lookups return empty sets after clear
- [✓] All 9 MutationObserver trait methods implemented and match trait signatures exactly
- [✓] `get_best_index` op-to-type mapping complete: Eq/Neq -> Hash, Gt/Gte/Lt/Lte -> Navigable, Like -> Inverted, Regex -> None, And/Or/Not -> None
- [✓] Type-mismatch guard present: returns `None` when registered index type does not match required type for the operation
- [✓] OrMap and OrTombstones records correctly skipped via `extract_rmpv` returning `None`
- [✓] `on_replication_put` no-ops when `populate_index` is false, indexes when true
- [✓] `IndexObserverFactory::register_map` is idempotent (pointer equality test confirms same Arc returned on repeated calls)
- [✓] `create_observer` returns `None` for unregistered maps
- [✓] `entry_count: u64` — no f64 for integer-semantic fields (constraint met)
- [✓] `IndexType` enum with `#[derive(Debug, Clone, PartialEq, Eq)]` — enums over strings for known value sets
- [✓] 3 files touched — within 5-file Rust language profile limit
- [✓] Build check: `cargo check` passes (0 errors)
- [✓] Lint check: `cargo clippy -- -D warnings` passes (0 warnings)
- [✓] Test check: 651 tests pass, 0 failures, 0 regressions
- [✓] No unnecessary `.clone()` calls beyond required Arc clones
- [✓] No `.unwrap()` or `.expect()` in production code paths
- [✓] No `unsafe` blocks
- [✓] No spec/bug/phase references in code comments — WHY-comments used throughout
- [✓] `Default` implemented via `fn default() -> Self { Self::new() }` on both `IndexRegistry` and `IndexObserverFactory`

**Minor:**
1. `on_update` has no dedicated test in `mutation_observer.rs`. Every other MutationObserver method has a direct test, but `on_update` (which calls `index.update` with old and new values) is untested. The code path is a trivial delegation, but a test would complete the coverage set.
2. `on_reset` also has no dedicated test; its implementation is identical to `on_clear`, so the behavioral coverage is present but the method is not exercised directly.
3. `on_put` receives `old_value: Option<&RecordValue>` but ignores it. When a key is re-put with a new value, the old value bucket is not cleaned up — correctness relies on `HashIndex`/`NavigableIndex`/`InvertedIndex` insert implementations handling the "key already present" case by removing from the old bucket first. This is SPEC-155a's responsibility, but future maintainers should be aware of this contract.

**Summary:** The implementation fully meets all acceptance criteria and requirements from the specification. All 9 MutationObserver methods are correctly implemented, the op-to-index-type mapping in `get_best_index` is complete and matches the spec, and all constraint checks pass. Build, lint, and test checks are all green with no regressions. Minor gaps in `on_update` and `on_reset` test coverage are the only items worth addressing.

### Fix Response v1 (2026-03-26)
**Applied:** All minor issues

**Fixes:**
1. [✓] Added `on_update_reindexes_changed_value` test — verifies old value removed from index and new value inserted after `on_update`
   - Commit: 474dd80
2. [✓] Added `on_reset_empties_all_indexes` test — verifies `on_reset` clears all index entries
   - Commit: 474dd80

**Skipped:**
3. [✗] `on_put` ignoring `old_value` — not a code fix; behavior is correct per spec (index `insert` handles key-already-present). Noted as implicit contract in review.

### Review v2 (2026-03-26 15:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Fix Verification:**
- [✓] Minor issue 1 fixed: `on_update_reindexes_changed_value` test added at line 390 of `mutation_observer.rs` — verifies old value ("alice") is removed from the hash index and new value ("bob") is inserted after `on_update`
- [✓] Minor issue 2 fixed: `on_reset_empties_all_indexes` test added at line 413 of `mutation_observer.rs` — inserts a record, calls `on_reset`, asserts `entry_count()` drops to 0
- [✓] Minor issue 3 skipped as documented — no action required, behavior is correct per spec

**Passed:**
- [✓] Both new tests pass: `on_update_reindexes_changed_value` and `on_reset_empties_all_indexes` verified by direct test run
- [✓] 69 index tests pass, 0 failures (up from 67 in v1 — 2 new tests added)
- [✓] Full suite: 653 tests pass (649 unit + 4 integration), 0 failures, 0 regressions
- [✓] Build check: `cargo check` passes (0 errors)
- [✓] Lint check: `cargo clippy -- -D warnings` passes (0 warnings)
- [✓] All AC1-AC4 remain fully met — no production code was changed, only tests added
- [✓] All v1 passing items remain valid

**Summary:** Both minor issues from Review v1 are correctly fixed. The two new tests directly exercise `on_update` and `on_reset`, completing coverage for all 9 MutationObserver methods. Build, lint, and all 653 tests are green. No further issues found.

---

## Completion

**Completed:** 2026-03-26
**Total Commits:** 4
**Review Cycles:** 2

### Outcome

Implemented IndexRegistry (per-map index management with DashMap) and IndexMutationObserver (wires index updates into the existing MutationObserver pipeline), with IndexObserverFactory for composable observer creation. 69 index tests, 653 total tests pass.

### Key Files

- `packages/server-rust/src/service/domain/index/registry.rs` — IndexRegistry with add/get/stats/get_best_index (op-to-type mapping)
- `packages/server-rust/src/service/domain/index/mutation_observer.rs` — IndexMutationObserver (all 9 MutationObserver methods) + IndexObserverFactory

### Patterns Established

None — followed existing MutationObserver/ObserverFactory pattern from MerkleMutationObserver.

### Deviations

1. `Value::Map` uses `BTreeMap` not `HashMap` — fixed test helper accordingly
2. Clippy `match-same-arms`: merged compound/Regex `None` branches into single arm
