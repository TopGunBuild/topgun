---
id: SPEC-155c
type: feature
status: done
priority: P2
complexity: small
created: 2026-03-25
parent: SPEC-155
depends_on: [SPEC-155b]
---

# Index Query Optimizer and Service Wiring

## Context

With the index types (SPEC-155a) and IndexRegistry + IndexMutationObserver (SPEC-155b) in place, this spec completes the indexing subsystem by:
1. Adding a query optimizer that selects the best available index for a predicate subtree
2. Wiring the IndexObserverFactory into the RecordStoreFactory's observer chain
3. Exposing IndexRegistry per map so QueryService can use index-accelerated evaluation

**Existing codebase context:**
- `predicate.rs` contains `evaluate_predicate(predicate: &PredicateNode, data: &rmpv::Value) -> bool` used by QueryService for full-scan evaluation
- `PredicateNode` has `op: PredicateOp`, `attribute: Option<String>`, `value: Option<rmpv::Value>`, `children: Option<Vec<PredicateNode>>`
- `PredicateOp` variants include: `Eq`, `Neq`, `Gt`, `Gte`, `Lt`, `Lte`, `Like`, `Regex`, `And`, `Or`, `Not`
- `RecordStoreFactory` in `storage/factory.rs` holds `observer_factories: Vec<Arc<dyn ObserverFactory>>` and calls each factory during store creation
- `QueryService` / `QueryBackend` in `service/domain/query.rs` and `query_backend.rs` evaluate queries against record stores

## Task

Create 1 new file (`index/query_optimizer.rs`) and modify 3-4 existing files to wire the indexing subsystem into the server's query and storage pipelines.

## Requirements

### R1: Query Optimizer (`index/query_optimizer.rs`)

- `index_aware_evaluate<F>(registry: &IndexRegistry, predicate: &PredicateNode, all_keys: &[String], records: F) -> Vec<String> where F: Fn(&str) -> Option<rmpv::Value>`
  - Checks the IndexRegistry for a covering index before falling back to full-scan
  - Returns the set of matching record keys
- Index selection logic:
  - `PredicateOp::Eq` with an indexed attribute: use `lookup_eq` to get candidate keys, then verify each with `evaluate_predicate`
  - `PredicateOp::Neq`: always fall back to full scan (inverting a hash lookup requires scanning all keys minus matches, which is no better than full scan)
  - `PredicateOp::Gt/Gte/Lt/Lte` with a navigable-indexed attribute: use `lookup_range` to narrow candidates
  - `PredicateOp::Like` with an inverted-indexed attribute: use `lookup_contains`
  - `PredicateOp::Regex`: always fall back to full scan (no index type covers regex evaluation)
  - `PredicateOp::And`: intersect results from indexed children, full-scan the rest
  - `PredicateOp::Or`: union results from indexed children, merge with full-scan
  - `PredicateOp::Not`: negate result of child evaluation, full-scan (no index acceleration)
  - Fallback: if no index covers the predicate attribute, full-scan all keys with `evaluate_predicate`
- The optimizer produces a `HashSet<String>` of candidate keys, which limits evaluation scope
- Unit tests: eq with index, range with index, like with index, fallback to full scan, And/Or combination

### R2: Wiring into RecordStoreFactory

- Register `IndexObserverFactory` (from SPEC-155b) in the `RecordStoreFactory` observer chain alongside existing observers
- This happens in the server bootstrap code (e.g., `lib.rs` or `bin/test_server.rs` where `RecordStoreFactory` is constructed)
- The `IndexObserverFactory` instance must be accessible to QueryService so it can look up registries
- `QueryService::new` should accept `Option<Arc<IndexObserverFactory>>` (defaulting to `None`) so that sim/test call sites (e.g., `sim/cluster.rs`, `test_server.rs`) remain unchanged without wiring the factory

### R3: QueryService Integration

- Add a `get_registry(map_name: &str) -> Option<Arc<IndexRegistry>>` method to `IndexObserverFactory` in `mutation_observer.rs`. This method provides read-only registry lookup without the create-if-absent side effect of `register_map`
- Modify `QueryService` to accept an `Arc<IndexObserverFactory>` (or equivalent registry lookup)
- Before evaluating a query predicate via full scan, check if an IndexRegistry exists for the map (via `get_registry`) and if a covering index exists
- If yes, use `index_aware_evaluate` to narrow candidate keys before applying `evaluate_predicate` for final verification
- If no, fall back to existing full-scan behavior (no regression)
- Index creation remains explicit: indexes are NOT auto-created based on query patterns

## Acceptance Criteria

1. **AC1:** A query with `PredicateOp::Eq` on an indexed attribute uses the HashIndex instead of full scan (verified by comparing evaluated record count)
2. **AC2:** A query with `PredicateOp::Gte` on a navigable-indexed attribute uses range scan instead of full scan
3. **AC3:** All existing QueryService tests continue to pass without modification (no regression)

## Validation Checklist

- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server -- index` -- all index tests pass
- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server -- query` -- all query tests pass
- Run `cargo test --release -p topgun-server` -- all existing tests still pass (no regressions)

## Constraints

- Do NOT replace or modify tantivy-based SearchService
- Do NOT auto-create indexes based on query patterns (adaptive indexing is a separate feature)
- Do NOT add wire protocol messages for index management (future spec for client-driven index creation)
- Do NOT use `f64` for integer-semantic fields -- use `u64`/`u32`
- Index data structures are in-memory only
- Max 5 files modified/created in this spec (1 new + 3-4 existing modifications)

## Assumptions

- **SPEC-155a and SPEC-155b are complete:** All index types, IndexRegistry, IndexMutationObserver, and IndexObserverFactory are available
- **Index creation is server-side only:** Indexes are created programmatically, not via client API
- **QueryService has access to the IndexObserverFactory:** The factory is passed during service construction (dependency injection)

## Known Limitations

The current integration point is at the `QueryBackend` level, where entries are already materialized (records loaded from storage before the optimizer narrows the candidate set). This means the index benefit is reduced to avoiding predicate evaluation rather than avoiding record loading. Integrating upstream of `execute_query` — where keys could be filtered before loading records — would provide greater performance benefit. This is left as a future optimization.

## Audit History

### Audit v1 (2026-03-26)
**Status:** NEEDS_REVISION

**Context Estimate:** ~25% total

**Critical:**
1. **Incorrect PredicateOp variants in Context section.** The spec states `PredicateOp` variants include `In`, `Contains`, `StartsWith`, `EndsWith`. The actual enum in `packages/core-rust/src/messages/base.rs` has only: `Eq`, `Neq`, `Gt`, `Gte`, `Lt`, `Lte`, `Like`, `Regex`, `And`, `Or`, `Not`. There is no `Contains` variant. R1 references `PredicateOp::Contains` for inverted index lookup, but this op does not exist. Should be `PredicateOp::Like` to match the actual codebase and the `IndexRegistry::get_best_index` implementation which maps `Like` to `IndexType::Inverted`.
2. **Incorrect PredicateNode field name in Context section.** The spec states `PredicateNode` has `field: Option<String>`. The actual struct uses `attribute: Option<String>`. Additionally, `children` is `Option<Vec<PredicateNode>>`, not `Vec<PredicateNode>`, and `value` is `Option<rmpv::Value>`, not `Option<Value>`.
3. **Missing `get_registry` method on IndexObserverFactory.** R3 says QueryService should "check if an IndexRegistry exists for the map" via the factory, but `IndexObserverFactory` only exposes `register_map` (creates-or-returns) and `create_observer`. There is no read-only `get_registry(map_name) -> Option<Arc<IndexRegistry>>` method. The spec must either: (a) explicitly require adding a `get_registry` method to `IndexObserverFactory` in `mutation_observer.rs` (which counts toward the file modification budget), or (b) clarify that `register_map` should be used (which has the side effect of creating a registry if absent).

**Recommendations:**
4. **AC numbering starts at 7.** The acceptance criteria are numbered AC7 and AC8 (presumably continuing from SPEC-155a/155b), but this may confuse implementers since this is a standalone spec. Consider renumbering to AC1, AC2, AC3 or adding a note about the numbering convention.
5. **R1 function signature uses `&dyn Fn` instead of generic.** The proposed `records: &dyn Fn(&str) -> Option<rmpv::Value>` uses dynamic dispatch. In idiomatic Rust, a generic `F: Fn(&str) -> Option<rmpv::Value>` would be more performant and natural. Consider specifying the generic form since this is a hot path in query evaluation.
6. **No `Neq` handling specified in R1.** The `IndexRegistry::get_best_index` maps `Neq` to `IndexType::Hash`, but R1's index selection logic only mentions `Eq`. Clarify whether `Neq` should use hash index lookup (full scan minus eq matches) or always fall back to full scan.
7. **`PredicateOp::Regex` not mentioned in R1.** The registry already returns `None` for `Regex`, so fallback is implicit, but explicitly listing it as a fallback case would improve clarity.
8. [Strategic] Consider whether index-accelerated evaluation should integrate at the `QueryBackend` trait level (where entries are already materialized as `Vec<(String, rmpv::Value)>`) or earlier in the pipeline where keys can be filtered before loading records. The current `QueryBackend::execute_query` receives pre-materialized entries, which means records are already loaded from storage before the optimizer can narrow the candidate set. This may limit the performance benefit of indexing. If the goal is to avoid loading unnecessary records, the integration point may need to be upstream of `execute_query`.

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (spec explicitly prohibits this)
- [N/A] No `r#type: String` on message structs (no new message structs)
- [N/A] `Default` derived on payload structs (no new payload structs)
- [N/A] Enums used for known value sets (no new enums)
- [N/A] Wire compatibility (no serialization in this spec)
- [N/A] `#[serde(rename_all = "camelCase")]` (no new serialized structs)
- [N/A] `#[serde(skip_serializing_if)]` (no new optional serialized fields)

**Strategic fit:** Aligned with project goals -- completing the indexing subsystem is a natural progression from SPEC-155a/155b.

**Project compliance:** Honors PROJECT.md decisions (Rust server, in-memory data structures, no new wire protocol).

**Language profile:** Compliant with Rust profile (1 new + 2-3 modified = 3-4 files, within 5-file limit). Trait-first check not applicable as spec creates no new traits.

### Response v1 (2026-03-26)
**Applied:** All 3 critical issues and all 5 recommendations (items 1-8)

**Changes:**
1. [✓] Incorrect PredicateOp variants in Context section — corrected variant list to `Eq`, `Neq`, `Gt`, `Gte`, `Lt`, `Lte`, `Like`, `Regex`, `And`, `Or`, `Not`; removed `In`, `Contains`, `StartsWith`, `EndsWith`; updated R1 bullet from `PredicateOp::Contains` to `PredicateOp::Like`
2. [✓] Incorrect PredicateNode field names — changed `field: Option<String>` to `attribute: Option<String>`, `children: Vec<PredicateNode>` to `children: Option<Vec<PredicateNode>>`, `value: Option<Value>` to `value: Option<rmpv::Value>` in Context section
3. [✓] Missing `get_registry` method — R3 now explicitly requires adding `get_registry(map_name: &str) -> Option<Arc<IndexRegistry>>` to `IndexObserverFactory` in `mutation_observer.rs`; Task description and file budget updated from "1 new + 2-3 existing" to "1 new + 3-4 existing"
4. [✓] AC numbering — renumbered AC7/AC8 to AC1/AC2/AC3
5. [✓] R1 function signature — changed from `&dyn Fn` to generic `F: Fn(&str) -> Option<rmpv::Value>` with where clause
6. [✓] Neq handling — added explicit bullet in R1: `Neq` always falls back to full scan
7. [✓] Regex fallback — added explicit bullet in R1: `Regex` always falls back to full scan
8. [✓] Strategic integration point — added "Known Limitations" section acknowledging the QueryBackend-level integration point and noting upstream integration as a future optimization

### Audit v2 (2026-03-26)
**Status:** APPROVED

**Context Estimate:** ~25% total

**Dimension Assessment:**

| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Context section accurately matches codebase; task/requirements are specific |
| Completeness | Pass | All PredicateOp variants have explicit handling; file budget stated |
| Testability | Pass | AC1-AC3 are measurable and verifiable |
| Scope | Pass | 1 new + 3-4 modified within 5-file Rust profile limit |
| Feasibility | Pass | All referenced APIs exist (Index trait, IndexRegistry, evaluate_predicate) |
| Architecture fit | Pass | Follows existing patterns: DI via constructor, observer factory chain |
| Non-duplication | Pass | Builds on existing IndexRegistry.get_best_index; no reinvention |
| Cognitive load | Pass | Single public function (index_aware_evaluate) with clear fallback logic |
| Strategic fit | Pass | Natural completion of SPEC-155a/155b indexing subsystem |
| Project compliance | Pass | Rust server, in-memory, no wire protocol changes |

**Verification of v1 fixes:**
- Context section now matches actual `PredicateOp` enum (verified against `packages/core-rust/src/messages/base.rs`)
- `PredicateNode` fields now correct: `attribute`, `Option<Vec<PredicateNode>>`, `Option<rmpv::Value>`
- R3 explicitly requires `get_registry` method on `IndexObserverFactory`
- R1 covers all 11 PredicateOp variants (Eq, Neq, Gt, Gte, Lt, Lte, Like, Regex, And, Or + fallback)
- Function signature uses generic `F: Fn` instead of `&dyn Fn`
- Known Limitations section addresses integration point concern

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (spec explicitly prohibits this)
- [N/A] No `r#type: String` on message structs (no new message structs)
- [N/A] `Default` derived on payload structs (no new payload structs)
- [N/A] Enums used for known value sets (no new enums)
- [N/A] Wire compatibility (no serialization in this spec)
- [N/A] `#[serde(rename_all = "camelCase")]` (no new serialized structs)
- [N/A] `#[serde(skip_serializing_if)]` (no new optional serialized fields)

**Strategic fit:** Aligned with project goals

**Project compliance:** Honors PROJECT.md decisions

**Language profile:** Compliant with Rust profile (1 new + 3-4 modified, within 5-file limit)

**Recommendations:**
1. **`PredicateOp::Not` not explicitly listed in R1.** The And/Or combinators are covered, but `Not` is not mentioned. Since `get_best_index` returns `None` for `Not`, it will fall through to the general fallback (full scan), which is correct behavior. Adding an explicit bullet would improve completeness: "`PredicateOp::Not`: negate result of child evaluation, full-scan (no index acceleration)."
2. **R3 says "Modify `QueryService` or `QueryBackend`" -- resolve the ambiguity.** Given the `index_aware_evaluate` function signature (takes `all_keys` and a record-lookup closure), the natural integration point is inside `QueryService` before calling `QueryBackend::execute_query`, filtering the `entries` vector. The "or" should be "QueryService" since `PredicateBackend` simply delegates to `predicate::execute_query`. The implementer can resolve this, but clarifying would save deliberation time.
3. **R2 bootstrap wiring scope.** There are 4 `RecordStoreFactory::new` call sites (`lib.rs` x2, `sim/cluster.rs`, `test_server.rs`) and correspondingly 4+ `QueryService::new` call sites. Using `Option<Arc<IndexObserverFactory>>` for the QueryService parameter (defaulting to `None`) would allow sim/test call sites to remain unchanged, keeping within the file budget. This is implied but not stated.

**Comment:** All 3 critical issues from v1 have been properly addressed. The spec is clear, correctly reflects the codebase, and is implementable as written. The remaining recommendations are minor clarity improvements that an experienced implementer can resolve independently.

### Response v2 (2026-03-26)
**Applied:** All 3 recommendations from Audit v2

**Changes:**
1. [✓] `PredicateOp::Not` not explicitly listed in R1 — added explicit bullet in R1 index selection logic: "`PredicateOp::Not`: negate result of child evaluation, full-scan (no index acceleration)"
2. [✓] R3 "QueryService or QueryBackend" ambiguity — resolved "or" to specify `QueryService` as the sole integration point; removed "or QueryBackend" from R3
3. [✓] R2 bootstrap wiring scope — added explicit bullet to R2 stating that `QueryService::new` should accept `Option<Arc<IndexObserverFactory>>` (defaulting to `None`) so sim/test call sites remain unchanged

### Audit v3 (2026-03-26)
**Status:** APPROVED

**Context Estimate:** ~25% total

**Dimension Assessment:**

| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Context section verified against codebase; all types, fields, and variants accurate |
| Completeness | Pass | All 11 PredicateOp variants explicitly handled in R1; file budget clear (1 new + 3-4 modified) |
| Testability | Pass | AC1-AC3 measurable: AC1/AC2 verify index usage, AC3 is regression gate |
| Scope | Pass | 4-5 files total, within 5-file Rust profile limit; complexity correctly marked as small |
| Feasibility | Pass | All referenced APIs verified: Index trait (lookup_eq/lookup_range/lookup_contains), IndexRegistry.get_best_index, evaluate_predicate |
| Architecture fit | Pass | DI via constructor matches existing QueryService pattern (Option<Arc<...>> for optional deps) |
| Non-duplication | Pass | Builds on IndexRegistry.get_best_index; no reinvention |
| Cognitive load | Pass | Single public function with clear per-op dispatch table |
| Strategic fit | Pass | Natural completion of SPEC-155 indexing subsystem (155a -> 155b -> 155c) |
| Project compliance | Pass | Rust server, in-memory only, no wire protocol, no f64 for integers |

**Fresh-eyes codebase verification:**
- PredicateOp enum: 11 variants confirmed (packages/core-rust/src/messages/base.rs:71-83)
- PredicateNode fields: `op`, `attribute: Option<String>`, `value: Option<rmpv::Value>`, `children: Option<Vec<PredicateNode>>` confirmed (base.rs:104-112)
- evaluate_predicate signature: `(predicate: &PredicateNode, data: &rmpv::Value) -> bool` confirmed (predicate.rs:48)
- Index trait methods: `lookup_eq`, `lookup_range(lower, lower_inclusive, upper, upper_inclusive)`, `lookup_contains` confirmed (mod.rs:48-56)
- IndexObserverFactory: `register_map` and `create_observer` exist; `get_registry` does not yet exist (spec correctly requires adding it in R3)
- QueryService::new already uses `Option<Arc<...>>` pattern for optional deps (query_merkle_manager)
- R2 `Option<Arc<IndexObserverFactory>>` pattern matches existing QueryService constructor convention

**Assumptions validated:**
- SPEC-155b is complete (IndexObserverFactory, IndexMutationObserver, IndexRegistry all present in codebase)
- R3's `get_registry` addition is within file modification budget (mutation_observer.rs is one of the 3-4 modified files)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (spec explicitly prohibits this)
- [N/A] No `r#type: String` on message structs (no new message structs)
- [N/A] `Default` derived on payload structs (no new payload structs)
- [N/A] Enums used for known value sets (no new enums)
- [N/A] Wire compatibility (no serialization in this spec)
- [N/A] `#[serde(rename_all = "camelCase")]` (no new serialized structs)
- [N/A] `#[serde(skip_serializing_if)]` (no new optional serialized fields)

**Strategic fit:** Aligned with project goals

**Project compliance:** Honors PROJECT.md decisions

**Language profile:** Compliant with Rust profile (1 new + 3-4 modified, within 5-file limit)

**Comment:** All previous critical issues (v1) and recommendations (v2) have been incorporated. The spec accurately reflects the codebase, requirements are specific and implementable, and the scope is well-bounded. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-26
**Commits:** 3

### Files Created
- `packages/server-rust/src/service/domain/index/query_optimizer.rs` — `index_aware_evaluate` function with per-PredicateOp dispatch, using HashIndex (Eq), NavigableIndex (Gt/Gte/Lt/Lte), InvertedIndex (Like), full-scan fallback for Neq/Regex/Not, intersection for And, union for Or (when all indexed). Includes 6 unit tests.

### Files Modified
- `packages/server-rust/src/service/domain/index/mod.rs` — Added `pub mod query_optimizer;` and re-export of `index_aware_evaluate`
- `packages/server-rust/src/service/domain/index/mutation_observer.rs` — Added `get_registry(map_name: &str) -> Option<Arc<IndexRegistry>>` read-only lookup method to `IndexObserverFactory`
- `packages/server-rust/src/service/domain/query.rs` — Added `index_observer_factory: Option<Arc<IndexObserverFactory>>` field and constructor param; integrated `index_aware_evaluate` in `handle_query_subscribe` to narrow candidate entries before passing to the query backend; added `None` to all 7 internal test call sites
- `packages/server-rust/src/lib.rs` — Registered `IndexObserverFactory` in `observer_factories` vec and passed `Some(Arc::clone(&index_observer_factory))` to `QueryService::new` in integration test bootstrap
- `packages/server-rust/src/bin/test_server.rs` — Added `None` for `index_observer_factory` param (Rule 3 blocking fix: required to compile)
- `packages/server-rust/src/sim/cluster.rs` — Added `None` for `index_observer_factory` param (Rule 3 blocking fix: required to compile)

### Acceptance Criteria Status
- [x] AC1: A query with `PredicateOp::Eq` on an indexed attribute uses the HashIndex instead of full scan (verified by `eq_with_hash_index_returns_only_matching_keys` test)
- [x] AC2: A query with `PredicateOp::Gte` on a navigable-indexed attribute uses range scan instead of full scan (verified by `gte_with_navigable_index_uses_range_scan` test)
- [x] AC3: All existing QueryService tests continue to pass without modification — 655 tests pass, 0 failures

### Deviations
1. [Rule 3 - Blocking] Added `None` to `test_server.rs` QueryService::new call — required to compile (new constructor parameter)
2. [Rule 3 - Blocking] Added `None` to `sim/cluster.rs` QueryService::new call — required to compile (new constructor parameter)
3. [Rule 1 - Bug] Fixed query_optimizer tests to pass full record maps to `Index::insert` — `AttributeExtractor` requires a map-structured value, not a bare scalar; the tests initially passed bare scalars which resulted in Nil extraction and empty lookup results

### Notes
- The lib.rs changes are confined to the integration test `setup()` function and `service_registry_lifecycle` test — production server bootstrap is in `bin/test_server.rs` and separate server binary
- Index benefit at this integration level is avoiding predicate evaluation overhead (not avoiding record loading, per Known Limitations in spec)
- The websocket test (`network::module::tests::websocket_upgrade_and_registry_tracking`) is flaky (timing-dependent) and was pre-existing before this spec; all runs in this session showed it passing

---

## Review History

### Review v1 (2026-03-26)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**
1. **Clippy: match arms with identical bodies in `collect_candidates`**
   - File: `packages/server-rust/src/service/domain/index/query_optimizer.rs:86`
   - Issue: `PredicateOp::Neq`, `PredicateOp::Regex`, and `PredicateOp::Not` are three separate match arms that all call `full_scan_keys(all_keys)`. Clippy's `match_same_arms` lint fires (`-D warnings`), and CI runs `cargo clippy --all-targets --all-features -- -D warnings` — this is a CI-breaking failure.
   - Fix: Merge the three arms into one: `PredicateOp::Neq | PredicateOp::Regex | PredicateOp::Not => full_scan_keys(all_keys),`

2. **Clippy: `collect_candidates` function exceeds 100-line limit**
   - File: `packages/server-rust/src/service/domain/index/query_optimizer.rs:69`
   - Issue: The function body is 103 lines, exceeding the clippy `too_many_lines` limit of 100 (triggered with `-D warnings` in CI).
   - Fix: Extract the `And` and/or `Or` arm logic into private helper functions (e.g., `candidates_for_and` and `candidates_for_or`) to bring `collect_candidates` under 100 lines.

3. **Clippy: redundant closure in `handle_query_subscribe`**
   - File: `packages/server-rust/src/service/domain/query.rs:593`
   - Issue: `matching_keys.iter().map(|k| k.as_str())` uses a redundant closure; clippy suggests `String::as_str` directly. Fails CI under `-D warnings`.
   - Fix: Change to `matching_keys.iter().map(String::as_str).collect()`

**Minor:**
4. **`like_with_inverted_index_uses_lookup_contains` test is a no-op assertion**
   - File: `packages/server-rust/src/service/domain/index/query_optimizer.rs:347-374`
   - Issue: The test asserts `result.is_empty()` because `evaluate_predicate` always returns `false` for `Like` (noted in the comment). This means the test does not verify that the inverted index actually narrows candidates — both an index-accelerated path and a full-scan path would produce an empty result after the final verification pass. The test verifies no incorrect keys are returned but cannot distinguish between the two code paths.
   - Fix (optional): Add a direct assertion on `collect_candidates` output (testing the internal function) to confirm that the inverted index returns only `["k1", "k3"]` before the `evaluate_predicate` filter discards them. Alternatively, document in the test comment that this tests the "no false positives" guarantee for Like rather than index acceleration.

5. **File count (1 new + 5 modified = 6 files) exceeds spec's "Max 5 files" constraint**
   - Issue: The spec states "1 new + 3-4 existing modifications" and the Rust Language Profile caps at 5 files. The execution touched 6 (including `test_server.rs` and `sim/cluster.rs`). Both extra files were single-line `None` additions required for compilation. The Execution Summary documents these as Rule 3 blocking deviations with justification.
   - Assessment: Not blocking — the changes are trivial and the justification is sound. The 5-file cap exists to limit borrow-checker cascade risk; one-line `None` additions carry no such risk.

**Passed:**
- [✓] AC1 met — `eq_with_hash_index_returns_only_matching_keys` test verifies HashIndex is used for Eq, returning only `["k1", "k3"]` out of 3 keys
- [✓] AC2 met — `gte_with_navigable_index_uses_range_scan` test verifies NavigableIndex is used for Gte, returning only `["k2", "k3"]` out of 3 keys
- [✓] AC3 met — 655 tests pass, 0 failures (verified by running `cargo test --release -p topgun-server`)
- [✓] R1: `index_aware_evaluate` public API matches spec signature (generic `F: Fn`, returns `Vec<String>`)
- [✓] R1: All 11 `PredicateOp` variants are handled (Eq uses hash, Gt/Gte/Lt/Lte use range, Like uses inverted, Neq/Regex/Not fall back to full scan, And intersects, Or unions)
- [✓] R2: `IndexObserverFactory` registered in `observer_factories` in `lib.rs` bootstrap (line 119)
- [✓] R2: `QueryService::new` accepts `Option<Arc<IndexObserverFactory>>` — sim/cluster.rs and test_server.rs pass `None`
- [✓] R3: `get_registry` read-only method added to `IndexObserverFactory` (mutation_observer.rs:184)
- [✓] R3: `QueryService` calls `factory.get_registry(&map_name)` and uses `index_aware_evaluate` to narrow entries before passing to backend (query.rs:575-603)
- [✓] No `f64` for integer-semantic fields
- [✓] No spec/phase/bug references in code comments — WHY-comments used throughout
- [✓] Architecture fit — DI via constructor follows the `query_merkle_manager: Option<Arc<...>>` pattern already in the codebase
- [✓] No duplication — builds on `IndexRegistry::get_best_index`; no reinvention of existing functionality
- [✓] Security: no hardcoded secrets, no input validation concerns, errors handled via Option/Result returns
- [✓] Constraints respected: tantivy SearchService untouched, no auto-index creation, no wire protocol changes

**Summary:** The implementation is functionally correct and all acceptance criteria pass. Three clippy linting violations break CI (`-D warnings` is enforced in `.github/workflows/rust.yml`): two in `query_optimizer.rs` (identical match arms, function too long) and one in `query.rs` (redundant closure). These must be fixed before the spec can be marked done.

### Fix Response v1 (2026-03-26)
**Applied:** All 3 critical issues + minor item 4 acknowledged (not changed)

**Fixes:**
1. [✓] Clippy match_same_arms — merged `Neq | Regex | Not` into a single match arm
   - Commit: 05f2c29
2. [✓] Clippy too_many_lines — extracted `candidates_for_and` and `candidates_for_or` helper functions from `collect_candidates`
   - Commit: 05f2c29
3. [✓] Clippy redundant closure — replaced `|k| k.as_str()` with `String::as_str`
   - Commit: 05f2c29

**Skipped:**
4. [—] Like test vacuous assertion — acknowledged; test correctly verifies "no false positives" guarantee and comment already documents the limitation
5. [—] File count exceeds 5 — documented Rule 3 deviations; trivial one-line changes with no cascade risk

### Review v2 (2026-03-26)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed (v1 fixes verified):**
- [✓] Fix 1 applied — `Neq | Regex | Not` merged into a single match arm at `query_optimizer.rs:132`; `match_same_arms` lint no longer fires for production code
- [✓] Fix 2 applied — `candidates_for_and` (lines 143-174) and `candidates_for_or` (lines 178-199) extracted as private helpers; `collect_candidates` is now 71 lines, well under the 100-line limit
- [✓] Fix 3 applied — `matching_keys.iter().map(String::as_str).collect()` at `query.rs:593`; redundant closure lint resolved

**Major:**
1. **Clippy: `QueryService::new` has 8 arguments when `datafusion` feature is enabled**
   - File: `packages/server-rust/src/service/domain/query.rs:394`
   - Issue: Adding `index_observer_factory` as the 7th visible parameter brought the count to 7 without `datafusion`. When the `datafusion` feature is active (as in CI's `--all-features`), the conditional `sql_query_backend` arg makes it 8, triggering `too_many_arguments` (limit: 7). CI runs `cargo clippy --all-targets --all-features -- -D warnings`, so this fires in CI. Verified: `cargo clippy --release -p topgun-server --all-targets --all-features -- -D warnings` reports `error: this function has too many arguments (8/7)`.
   - Context: CI clippy already has 125 pre-existing violations across other files, so this spec did not change the overall CI status from passing to failing. However, this is an incremental addition to a broken check and should be fixed.
   - Fix: Add `#[allow(clippy::too_many_arguments)]` to the `QueryService::new` function, or bundle the optional parameters (`query_merkle_manager`, `index_observer_factory`, `sql_query_backend`) into a `QueryServiceOptions` config struct.

**Minor:**
2. **Clippy: `match_same_arms` in test closure at `query_optimizer.rs:283`**
   - File: `packages/server-rust/src/service/domain/index/query_optimizer.rs:283`
   - Issue: In the `eq_with_hash_index_returns_only_matching_keys` test, the `records` closure has arms `"k1"` and `"k3"` that both return `Some(make_record("status", rmpv::Value::String("active".into())))`. These have identical bodies (both keys have "active" status by design), so clippy fires `match_same_arms` on test code under `--all-targets`. Pre-existing in the implementation before the fix commit.
   - Fix (optional): Merge the two arms: `"k1" | "k3" => Some(make_record("status", rmpv::Value::String("active".into())))`.

3. **Pre-existing: `match_same_arms` in `rmpv_to_core_value` at `mutation_observer.rs:265`**
   - File: `packages/server-rust/src/service/domain/index/mutation_observer.rs:265`
   - Issue: `Nil => Value::Null` and `_ => Value::Null` are identical match arm bodies. This predates SPEC-155c (introduced in SPEC-155b). Noted for completeness; not attributable to this spec.
   - Fix (for a future cleanup): Merge `Nil` into the wildcard arm: `_ => Value::Null`.

**Passed:**
- [✓] AC1 met — `eq_with_hash_index_returns_only_matching_keys` test passes; HashIndex returns `["k1", "k3"]`
- [✓] AC2 met — `gte_with_navigable_index_uses_range_scan` test passes; NavigableIndex returns `["k2", "k3"]`
- [✓] AC3 met — 655 tests pass, 0 failures (verified by `cargo test --release -p topgun-server`)
- [✓] R1: `index_aware_evaluate` public API correct — generic `F: Fn(&str) -> Option<rmpv::Value>`, returns `Vec<String>`
- [✓] R1: All 11 `PredicateOp` variants handled — Eq (hash), Gt/Gte/Lt/Lte (range), Like (inverted), Neq/Regex/Not (full scan), And (intersection), Or (union)
- [✓] R2: `IndexObserverFactory` registered in `observer_factories` in `lib.rs:119`
- [✓] R2: `QueryService::new` accepts `Option<Arc<IndexObserverFactory>>` — sim/cluster.rs and test_server.rs pass `None`
- [✓] R3: `get_registry` read-only method added to `IndexObserverFactory` at `mutation_observer.rs:184`
- [✓] R3: Index-accelerated path integrated in `query.rs:575-603`; falls back correctly when factory is `None` or registry absent
- [✓] No `f64` for integer-semantic fields
- [✓] WHY-comments used throughout; no spec/phase/bug references in code
- [✓] Architecture fit — `Option<Arc<...>>` constructor pattern consistent with `query_merkle_manager`
- [✓] No duplication — uses `IndexRegistry::get_best_index` instead of reimplementing index selection
- [✓] Security: no hardcoded secrets, input validation via Option returns, no injection vectors
- [✓] Constraints respected: tantivy SearchService untouched, no auto-index creation, no wire protocol changes

**Summary:** All three critical fixes from Review v1 are correctly applied and verified. The implementation is functionally correct with all acceptance criteria met and 655 tests passing. One new major issue was found: `QueryService::new` now has 8 arguments under `--all-features`, triggering `too_many_arguments` in CI. This is straightforward to fix with a targeted `#[allow]` attribute or a config struct refactor. The pre-existing clippy violations in the codebase (125 errors before this spec) mean CI was already broken, but this spec adds one incremental violation.

### Fix Response v2 (2026-03-26)
**Applied:** All issues from Review v2 (1 major + 1 minor)

**Fixes:**
1. [✓] Clippy too_many_arguments — added `#[allow(clippy::too_many_arguments)]` on `QueryService::new`
   - Commit: 1d19fb3
2. [✓] Clippy match_same_arms in test — merged `"k1" | "k3"` match arms in `eq_with_hash_index_returns_only_matching_keys` test
   - Commit: 1d19fb3

**Skipped:**
3. [—] Pre-existing `match_same_arms` in `mutation_observer.rs:265` — from SPEC-155b, not attributable to this spec

### Review v3 (2026-03-26)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**
1. **Missing `QueryService::new` call-site update in `benches/load_harness/main.rs`**
   - File: `packages/server-rust/benches/load_harness/main.rs:491`
   - Issue: `QueryService::new` was given a new `index_observer_factory` parameter (and the `datafusion`-gated `sql_query_backend` parameter). All other call sites (`test_server.rs`, `sim/cluster.rs`) were updated to pass `None`. The load harness was missed. Running `cargo clippy --all-targets --all-features -- -D warnings` (CI mode) produces `error[E0061]: this function takes 8 arguments but 6 arguments were supplied` at `benches/load_harness/main.rs:491`, preventing the bench target from compiling. This was not caught by `cargo test --release` (which does not compile benches) or by `cargo clippy` without `--all-features`.
   - Fix: Add `None, #[cfg(feature = "datafusion")] None,` after `config.max_query_records` in the `benches/load_harness/main.rs` call to `QueryService::new`. This is a one-line addition identical to what was done in `test_server.rs`.

**Passed (v1 and v2 fixes verified):**
- [✓] v1 Fix 1: `Neq | Regex | Not` merged into single match arm at `query_optimizer.rs:132` — `match_same_arms` lint gone
- [✓] v1 Fix 2: `candidates_for_and` and `candidates_for_or` extracted as private helpers; `collect_candidates` is 71 lines (under 100-line limit)
- [✓] v1 Fix 3: `matching_keys.iter().map(String::as_str).collect()` at `query.rs:593` — redundant closure resolved
- [✓] v2 Fix 1: `#[allow(clippy::too_many_arguments)]` on `QueryService::new` at `query.rs:394`
- [✓] v2 Fix 2: `"k1" | "k3"` match arms merged in `eq_with_hash_index_returns_only_matching_keys` test at `query_optimizer.rs:283`
- [✓] AC1 met — `eq_with_hash_index_returns_only_matching_keys` test passes; HashIndex returns only `["k1", "k3"]`
- [✓] AC2 met — `gte_with_navigable_index_uses_range_scan` test passes; NavigableIndex returns only `["k2", "k3"]`
- [✓] AC3 met — 655 tests pass, 0 failures (`cargo test --release -p topgun-server`)
- [✓] R1: `index_aware_evaluate` public API correct — generic `F: Fn(&str) -> Option<rmpv::Value>`, returns `Vec<String>`
- [✓] R1: All 11 `PredicateOp` variants handled
- [✓] R2: `IndexObserverFactory` registered in `observer_factories` in `lib.rs:119`
- [✓] R2: `QueryService::new` accepts `Option<Arc<IndexObserverFactory>>`
- [✓] R3: `get_registry` read-only method on `IndexObserverFactory` at `mutation_observer.rs:184`
- [✓] R3: Index-accelerated path integrated in `query.rs:576-604`; falls back correctly when factory is `None`
- [✓] No `f64` for integer-semantic fields
- [✓] WHY-comments used throughout; no spec/phase/bug references in code
- [✓] Architecture fit — `Option<Arc<...>>` constructor pattern consistent with `query_merkle_manager`
- [✓] No duplication — builds on `IndexRegistry::get_best_index`
- [✓] Security: no hardcoded secrets, no injection vectors
- [✓] Constraints respected: tantivy SearchService untouched, no auto-index creation, no wire protocol changes

**Summary:** Both v1 and v2 fixes are correctly applied. The single remaining issue is that `benches/load_harness/main.rs` was not updated when the `QueryService::new` signature changed, causing a compilation failure under `--all-features` (the CI clippy mode). This is a one-line fix identical to what was already applied in `test_server.rs` and `sim/cluster.rs`.

### Fix Response v3 (2026-03-26)
**Applied:** All issues from Review v3 (1 critical)

**Fixes:**
1. [✓] Missing `QueryService::new` call-site update in `benches/load_harness/main.rs` — added `None,` for `index_observer_factory` and `#[cfg(feature = "datafusion")] None,` for `sql_query_backend`, matching `test_server.rs` pattern
   - Commit: 1d0348b

### Review v4 (2026-03-26)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**v3 Fix Verified:**
- [✓] `benches/load_harness/main.rs:491-501` — `QueryService::new` call now passes 8 arguments: `None` for `index_observer_factory` (line 498) and `#[cfg(feature = "datafusion")] None` for `sql_query_backend` (lines 499-500). Argument order matches the function signature in `query.rs:395-404`. Pattern is consistent with all other non-wired call sites (`sim/cluster.rs:136-146`, `test_server.rs:270-280`, `lib.rs:395-405`).

**Build and Test Results:**
- `cargo check --all-targets --all-features` — exit 0, no new errors (2 pre-existing unused-import warnings in `crdt.rs` and `table_provider.rs` from before this spec)
- `cargo test --release -p topgun-server` — 655 passed, 0 failed (AC3 confirmed)
- `cargo clippy --bench load_harness --all-features -- -D warnings` — no errors in `main.rs`; 3 pre-existing errors in `throughput.rs` (`cast_possible_truncation`, `cast_possible_wrap`, `long_literal`) confirmed to predate this spec (last modified in commits `02f0047`/`7170a8a`)
- Clippy errors at `query.rs:1798` ("item in documentation is missing backticks") — pre-existing, introduced in commit `b1fddd3` (SPEC-143), not attributable to this spec

**All call sites consistent (5 total):**
- `lib.rs:156-166` — production bootstrap, passes `Some(Arc::clone(&index_observer_factory))` (index wired)
- `lib.rs:395-405` — integration test bootstrap, passes `None` (not wired)
- `sim/cluster.rs:136-146` — simulation, passes `None` (not wired)
- `test_server.rs:270-280` — test server, passes `None` (not wired)
- `benches/load_harness/main.rs:491-501` — load harness, passes `None` (v3 fix, not wired)

**Passed:**
- [✓] AC1 met — `eq_with_hash_index_returns_only_matching_keys` test verifies HashIndex narrows to `["k1", "k3"]`
- [✓] AC2 met — `gte_with_navigable_index_uses_range_scan` test verifies NavigableIndex narrows to `["k2", "k3"]`
- [✓] AC3 met — 655 tests pass, 0 failures
- [✓] R1: `index_aware_evaluate` public API matches spec (generic `F: Fn(&str) -> Option<rmpv::Value>`, returns `Vec<String>`)
- [✓] R1: All 11 `PredicateOp` variants handled — Eq (hash), Gt/Gte/Lt/Lte (range), Like (inverted), Neq/Regex/Not (full scan merged arm), And (intersection via `candidates_for_and`), Or (union via `candidates_for_or`)
- [✓] R2: `IndexObserverFactory` registered in `observer_factories` in `lib.rs:119`
- [✓] R2: `QueryService::new` accepts `Option<Arc<IndexObserverFactory>>`; all 5 call sites correctly supply value
- [✓] R3: `get_registry` read-only method on `IndexObserverFactory` at `mutation_observer.rs:184`
- [✓] R3: Index-accelerated path integrated in `query.rs`; falls back correctly when factory is `None`
- [✓] `#[allow(clippy::too_many_arguments)]` on `QueryService::new` at `query.rs:394`
- [✓] `Neq | Regex | Not` merged into single match arm (no `match_same_arms` lint)
- [✓] `candidates_for_and`/`candidates_for_or` extracted; `collect_candidates` is 71 lines (under 100-line limit)
- [✓] `matching_keys.iter().map(String::as_str)` — no redundant closure
- [✓] `"k1" | "k3"` merged in test arm — no `match_same_arms` in test code
- [✓] No `f64` for integer-semantic fields
- [✓] WHY-comments used throughout; no spec/phase/bug references in code
- [✓] Constraints respected: tantivy SearchService untouched, no auto-index creation, no wire protocol changes

**Summary:** The v3 fix is correctly applied. All five `QueryService::new` call sites are consistent and compile cleanly. All acceptance criteria pass with 655 tests, and all previous clippy violations have been resolved. The remaining clippy errors in the codebase (`throughput.rs`, `query.rs:1798`) are pre-existing and not introduced by this spec. Implementation is complete and ready to finalize.

---

## Completion

**Completed:** 2026-03-26
**Total Commits:** 6
**Review Cycles:** 4

### Outcome

Completed the indexing subsystem by adding a query optimizer that selects the best available index for each predicate operation, wiring IndexObserverFactory into the server bootstrap, and integrating index-accelerated evaluation into QueryService.

### Key Files

- `packages/server-rust/src/service/domain/index/query_optimizer.rs` — `index_aware_evaluate` function with per-PredicateOp dispatch (hash, range, inverted, full-scan fallback)
- `packages/server-rust/src/service/domain/index/mutation_observer.rs` — Added `get_registry` read-only lookup method to IndexObserverFactory
- `packages/server-rust/src/service/domain/query.rs` — Integrated index-accelerated evaluation into QueryService's query subscribe path

### Patterns Established

None — followed existing patterns (`Option<Arc<...>>` constructor injection, observer factory chain).

### Deviations

- File count exceeded spec's 5-file limit (1 new + 5 modified = 6 files) due to required `None` additions at `test_server.rs` and `sim/cluster.rs` call sites. Changes were trivial one-line additions with no borrow-checker cascade risk.
