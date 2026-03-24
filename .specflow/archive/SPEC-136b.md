---
id: SPEC-136b
type: feature
status: done
priority: P1
complexity: small
parent: SPEC-136
depends_on: [SPEC-136a]
created: 2026-03-21
source: TODO-070
---

# Shapes: ShapeEvaluator Module and ShapeRegistry

## Context

SPEC-136a established the `SyncShape` type (with `PredicateNode`-based filter), shape wire messages, and `Operation` variants. This sub-spec implements the evaluation logic and the server-side registry for tracking active shapes per connection.

The `ShapeEvaluator` is a module of free functions (not a trait) -- `matches()`, `project()`, and `apply_shape()` -- that evaluate shape filters against records using the existing `evaluate_predicate()` function from the predicate engine. No trait is needed because there is only one evaluation strategy and the functions are pure (no state).

The `ShapeRegistry` is a `DashMap`-based concurrent data structure that tracks which shapes are active on which connections, following the same pattern as the existing `QueryRegistry`.

**Audit note:** `shapes_for_map()` requires a linear scan of the DashMap since it is keyed by `shape_id`, not `map_name`. This is acceptable for small shape counts (typical: tens to low hundreds per server). If shape counts grow large, a secondary index (`DashMap<String, Vec<String>>` mapping `map_name` to `shape_id`s) can be added as an optimization. This optimization is deferred.

## Task

Implement the `ShapeEvaluator` module (free functions for filter matching, field projection, and combined evaluation) and the `ShapeRegistry` data structure with unit tests.

## Requirements

### R1: ShapeEvaluator module

**File:** `packages/server-rust/src/service/domain/shape_evaluator.rs` (new)

Three free functions (not a trait):

- `fn matches(shape: &SyncShape, record: &rmpv::Value) -> bool` -- evaluates the shape's `filter: Option<PredicateNode>` against a record using `evaluate_predicate()`. If `filter` is `None`, the record always matches.
- `fn project(fields: &[String], record: &rmpv::Value) -> rmpv::Value` -- strips non-projected fields from a Map value, returns projected subset. If the record is not a Map, returns it unchanged.
- `fn apply_shape(shape: &SyncShape, record: &rmpv::Value) -> Option<rmpv::Value>` -- combines match + project; returns `None` if filtered out, `Some(projected_value)` if matching. If `fields` is `None`, no projection is applied. The `key` parameter has been removed: neither `matches()` nor `project()` uses it, and callers that need to build a `ShapeRecord` can pass the key they already hold directly.

Shape evaluation MUST be synchronous (no async in the hot filter path).

**Unit tests:**

- `matches` with `None` filter returns `true`
- `matches` with EQ filter returns `true` for matching record, `false` for non-matching
- `matches` with AND compound filter
- `project` strips non-projected fields from a Map value
- `project` on non-Map value returns unchanged
- `apply_shape` returns `None` for non-matching record
- `apply_shape` returns projected value for matching record
- `apply_shape` with `fields: None` returns full value (no projection)

### R2: ShapeRegistry

**File:** `packages/server-rust/src/service/domain/shape.rs` (new -- registry only, ShapeService added in SPEC-136c)

Data structures:

- `ActiveShape`: `shape: SyncShape`, `connection_id: u64`
  - MUST derive `#[derive(Debug, Clone)]` -- `get()`, `shapes_for_map()`, and `shapes_for_connection()` all return owned `ActiveShape` values cloned out of `DashMap` refs; without `Clone` these methods will not compile.
  - Note: `map_name` is intentionally omitted from `ActiveShape` -- access via `active_shape.shape.map_name`
- `ShapeRegistry`: wraps `DashMap<String, ActiveShape>` keyed by `shape_id`

Methods on `ShapeRegistry`:

- `fn new() -> Self`
- `fn register(&self, shape_id: String, connection_id: u64, shape: SyncShape) -> Result<(), ShapeRegistryError>` -- returns error if `shape_id` already exists
- `fn unregister(&self, shape_id: &str) -> Option<ActiveShape>` -- removes and returns
- `fn unregister_all_for_connection(&self, connection_id: u64) -> Vec<String>` -- returns removed shape_ids
- `fn shapes_for_map(&self, map_name: &str) -> Vec<(String, ActiveShape)>` -- all active shapes targeting a map (linear scan, see audit note above)
- `fn shapes_for_connection(&self, connection_id: u64) -> Vec<(String, ActiveShape)>` -- all shapes for a connection
- `fn get(&self, shape_id: &str) -> Option<ActiveShape>` -- lookup by shape_id

`ShapeRegistryError` enum:
- MUST derive `#[derive(Debug, thiserror::Error)]` -- the project uses `thiserror = "2"` (confirmed in Cargo.toml)
- `DuplicateShapeId(String)` -- shape_id already registered; MUST annotate `#[error("Shape ID already registered: {0}")]`

**Unit tests:**

- Register and retrieve a shape
- Register duplicate shape_id returns error
- Unregister returns the removed shape
- Unregister non-existent returns None
- `unregister_all_for_connection` removes all shapes for a connection, returns shape_ids
- `shapes_for_map` returns only shapes targeting the specified map
- `shapes_for_connection` returns only shapes for the specified connection
- Multiple shapes from different connections on the same map

### R3: Module registration

**File:** `packages/server-rust/src/service/domain/mod.rs` (modify)

Add:
```rust
pub mod shape;
pub mod shape_evaluator;
```

## Acceptance Criteria

1. `shape_evaluator::matches()` correctly evaluates `PredicateNode` filters against `rmpv::Value` records
2. `shape_evaluator::matches()` returns `true` when filter is `None`
3. `shape_evaluator::project()` returns only projected fields from a Map value
4. `shape_evaluator::apply_shape()` combines matching and projection correctly
5. `ShapeRegistry` registers, retrieves, and unregisters shapes correctly
6. `ShapeRegistry.register()` returns `DuplicateShapeId` error for duplicate shape_ids
7. `ShapeRegistry.unregister_all_for_connection()` removes all shapes for a given connection
8. `ShapeRegistry.shapes_for_map()` returns all shapes targeting a specific map
9. All unit tests pass for both evaluator and registry
10. `cargo test --release -p topgun-server` passes (all existing + new tests)

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all tests pass
2. Verify `shape_evaluator.rs` functions are synchronous (no async)
3. Verify `ShapeRegistry` uses `DashMap` for concurrent access

## Constraints

- Shape evaluation MUST be synchronous (no async in the hot filter path)
- Do NOT add `ShapeService` Tower service in this sub-spec -- that is SPEC-136c
- Do NOT add DataFusion dependency -- shape evaluation uses `PredicateNode` + `evaluate_predicate()` only
- Max 5 Rust files

## Assumptions

1. Shapes are LWW-Map only (not OR-Map) for the initial implementation.
2. No nested field paths in filters -- predicates operate on top-level fields only (matching existing `PredicateNode` capabilities).
3. Shape counts per server are small enough (tens to low hundreds) that linear scan in `shapes_for_map` is acceptable.

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `shape_evaluator.rs`: `matches`, `project`, `apply_shape` functions + unit tests | -- | ~10% |
| G2 | 1 | `shape.rs`: `ActiveShape`, `ShapeRegistry`, `ShapeRegistryError` + unit tests | -- | ~10% |
| G3 | 2 | `mod.rs` registration | G1, G2 | ~2% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-21)
**Status:** APPROVED

**Context Estimate:** ~27% total (22% spec + 5% overhead)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~27% | <=50% | OK |
| Largest task group | ~10% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Dimensions:**
- Clarity: Good -- context, task, and requirements are well-structured
- Completeness: Good -- all files, methods, and test cases enumerated
- Testability: Good -- each acceptance criterion is verifiable
- Scope: Good -- clear boundaries (no ShapeService, no DataFusion)
- Feasibility: Good -- uses existing `evaluate_predicate()` and `DashMap` patterns
- Architecture fit: Good -- follows `QueryRegistry` pattern from existing codebase
- Non-duplication: Good -- reuses `evaluate_predicate()` rather than reimplementing
- Cognitive load: Good -- simple free functions + familiar DashMap registry
- Strategic fit: Aligned with project goals (partial replication is table-stakes feature)
- Project compliance: Honors PROJECT.md decisions (MsgPack, no new deps, Rust patterns)

**Language profile:** Compliant with Rust profile (3 files <= 5 max, no trait-first violation for small spec)

**Recommendations:**
1. `apply_shape` includes a `key: &str` parameter that is not used by either `matches()` or `project()`. Consider removing it or documenting its purpose (e.g., future limit tracking, caller convenience for building `ShapeRecord`). Harmless but may confuse implementers.
2. `ActiveShape` should explicitly specify `#[derive(Debug, Clone)]` since methods like `get()`, `shapes_for_map()`, and `shapes_for_connection()` return owned `ActiveShape` values extracted from `DashMap` refs. Without `Clone`, these methods cannot compile. The implementer will likely infer this, but the spec should be explicit for unambiguous implementation.

**Comment:** Well-scoped spec with clear requirements. Both modules are independent (Wave 1 parallel), making this efficient to implement. Good use of existing patterns (QueryRegistry for ShapeRegistry, evaluate_predicate for ShapeEvaluator).

### Response v1 (2026-03-21)
**Applied:** Both recommendations from Audit v1

**Changes:**
1. [✓] `apply_shape` `key: &str` parameter removed — signature changed to `fn apply_shape(shape: &SyncShape, record: &rmpv::Value) -> Option<rmpv::Value>` with inline rationale: neither `matches()` nor `project()` uses the key, and callers building a `ShapeRecord` already hold the key independently.
2. [✓] `ActiveShape` explicitly requires `#[derive(Debug, Clone)]` — added to R2 with explanation that `get()`, `shapes_for_map()`, and `shapes_for_connection()` return owned cloned values from `DashMap` refs; without `Clone` these methods will not compile.

### Audit v2 (2026-03-21 re-audit)
**Status:** APPROVED

**Context Estimate:** ~27% total (22% spec + 5% overhead)

**Verification:** Both v1 recommendations confirmed applied. Spec revised correctly.

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`connection_id: u64` correct)
- [x] No `r#type: String` on message structs (N/A -- internal structs)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- `ActiveShape` has no optional fields)
- [x] Enums used for known value sets (`ShapeRegistryError` is enum)
- [x] Wire compatibility (N/A -- internal structs, not serialized)
- [x] `#[serde(rename_all = "camelCase")]` (N/A -- internal structs not serialized over wire)
- [x] `#[serde(skip_serializing_if...)]` on Option fields (N/A -- no Option fields on new structs)

**Dimensions:** All 10 dimensions pass (unchanged from v1 -- spec quality improved with revisions).

**Strategic fit:** Aligned with project goals (partial replication is table-stakes feature per PROJECT.md).

**Project compliance:** Honors PROJECT.md decisions (MsgPack, no new deps, Rust patterns, DashMap).

**Language profile:** Compliant with Rust profile (3 files <= 5 max, no trait-first concern for small spec with no traits).

**Recommendations:**
3. `ShapeRegistryError` should specify `#[derive(Debug, thiserror::Error)]` with `#[error("Shape ID already registered: {0}")]` on `DuplicateShapeId(String)`. The project uses `thiserror` (Cargo.toml confirms `thiserror = "2"`). Implementer will likely infer this, but explicit specification removes ambiguity.

**Comment:** Clean spec after v1 revisions. Both recommendations applied correctly. Ready for implementation.

### Response v2 (2026-03-21)
**Applied:** Recommendation 3 from Audit v2

**Changes:**
1. [✓] `ShapeRegistryError` derive and error attribute specified — R2 `ShapeRegistryError` enum updated to require `#[derive(Debug, thiserror::Error)]` with rationale (project uses `thiserror = "2"`), and `DuplicateShapeId(String)` annotated with `#[error("Shape ID already registered: {0}")]`.

### Audit v3 (2026-03-21 re-audit)
**Status:** APPROVED

**Context Estimate:** ~27% total (22% spec + 5% overhead)

**Verification:** Recommendation 3 from Audit v2 confirmed applied. `ShapeRegistryError` now specifies `#[derive(Debug, thiserror::Error)]` and `#[error(...)]` annotation.

**Codebase verification:**
- `evaluate_predicate` confirmed as `pub fn evaluate_predicate(predicate: &PredicateNode, data: &rmpv::Value) -> bool` in `predicate.rs` -- accessible from `shape_evaluator.rs`
- `SyncShape` confirmed in `core-rust/src/schema.rs` with expected fields (`shape_id`, `map_name`, `filter: Option<PredicateNode>`, `fields: Option<Vec<String>>`)
- `QueryRegistry` confirmed as existing DashMap-based pattern in `query.rs` -- ShapeRegistry follows same approach
- `thiserror = "2"` confirmed in `Cargo.toml`
- `mod.rs` has clear insertion points for new module declarations

**All dimensions pass.** No new issues found. All three previous recommendations have been incorporated.

**Comment:** Spec is clean and ready for implementation. Three rounds of revision have addressed all identified concerns. Function signatures, derive macros, error types, and test cases are all explicitly specified.

## Execution Summary

**Executed:** 2026-03-21
**Mode:** orchestrated (sequential fallback -- subagent CLI unavailable in shell)
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Created
- `packages/server-rust/src/service/domain/shape_evaluator.rs` -- ShapeEvaluator module (matches, project, apply_shape) + 8 unit tests
- `packages/server-rust/src/service/domain/shape.rs` -- ActiveShape, ShapeRegistry, ShapeRegistryError + 8 unit tests

### Files Modified
- `packages/server-rust/src/service/domain/mod.rs` -- added `pub mod shape;` and `pub mod shape_evaluator;`

### Acceptance Criteria Status
- [x] 1. `shape_evaluator::matches()` correctly evaluates `PredicateNode` filters against `rmpv::Value` records
- [x] 2. `shape_evaluator::matches()` returns `true` when filter is `None`
- [x] 3. `shape_evaluator::project()` returns only projected fields from a Map value
- [x] 4. `shape_evaluator::apply_shape()` combines matching and projection correctly
- [x] 5. `ShapeRegistry` registers, retrieves, and unregisters shapes correctly
- [x] 6. `ShapeRegistry.register()` returns `DuplicateShapeId` error for duplicate shape_ids
- [x] 7. `ShapeRegistry.unregister_all_for_connection()` removes all shapes for a given connection
- [x] 8. `ShapeRegistry.shapes_for_map()` returns all shapes targeting a specific map
- [x] 9. All unit tests pass for both evaluator and registry (18 new tests)
- [x] 10. `cargo test --release -p topgun-server` passes (582 + 4 integration = all pass)

### Deviations
- None

---

## Review History

### Review v1 (2026-03-21)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**
1. **Clippy check fails with 13 errors in new files**
   - Files: `packages/server-rust/src/service/domain/shape.rs` (12 errors), `packages/server-rust/src/service/domain/shape_evaluator.rs` (1 error)
   - Issue: `cargo clippy --release -p topgun-server -- -D warnings` exits non-zero. The project maintains clippy-clean status (confirmed by STATE.md: "clippy-clean" noted for each prior spec). The Execution Summary incorrectly claims "clippy-clean". Specific errors:
     - `shape.rs:17,26,35,38,58,79,84,123` — "item in documentation is missing backticks" (8 occurrences): identifiers like `get()`, `shapes_for_map()`, `shapes_for_connection()`, `shape_id`, `map_name`, `filter`, `fields`, `limit`, `DashMap`, `QueryRegistry` used in doc comments without backtick wrapping
     - `shape.rs:80,85` — "this method could have a `#[must_use]` attribute": `unregister()` returns `Option<ActiveShape>` and `unregister_all_for_connection()` returns `Vec<String>` — both non-trivial return values that callers should not silently ignore
     - `shape_evaluator.rs:40-41` — "this `map_or` can be simplified": `.map_or(false, |key_str| ...)` should be `.is_some_and(|key_str| ...)`
   - Fix: Wrap bare identifiers in doc comments with backticks; add `#[must_use]` to `unregister` and `unregister_all_for_connection`; replace `.map_or(false, ...)` with `.is_some_and(...)` in `shape_evaluator.rs:41`

**Passed:**
- [✓] AC1: `shape_evaluator::matches()` correctly evaluates `PredicateNode` filters — EQ, GTE, AND compound all tested
- [✓] AC2: `shape_evaluator::matches()` returns `true` when filter is `None` — `matches_none_filter_returns_true` test passes
- [✓] AC3: `shape_evaluator::project()` returns only projected fields — `project_strips_non_projected_fields` test passes
- [✓] AC4: `apply_shape()` combines matching + projection correctly — 3 tests covering None-filter/full-value, filter-out, and projected result
- [✓] AC5: `ShapeRegistry` registers, retrieves, and unregisters — all registry methods implemented and tested
- [✓] AC6: `register()` returns `DuplicateShapeId` error — `register_duplicate_returns_error` test passes
- [✓] AC7: `unregister_all_for_connection()` removes all shapes for a connection — test verifies correct removal and return of shape_ids
- [✓] AC8: `shapes_for_map()` returns all shapes targeting a map — `shapes_for_map_filters_correctly` test passes
- [✓] AC9: All 18 unit tests pass — `cargo test --release -p topgun-server` reports 582 passed, 0 failed
- [✓] R1: Three free functions (`matches`, `project`, `apply_shape`) — all synchronous, no async
- [✓] R2: `ActiveShape` derives `Debug + Clone`; `ShapeRegistryError` derives `Debug + thiserror::Error` with correct `#[error(...)]`
- [✓] R3: `pub mod shape;` and `pub mod shape_evaluator;` registered in `mod.rs`
- [✓] Constraint: No async in evaluation path
- [✓] Constraint: No DataFusion dependency added
- [✓] Constraint: Max 5 files (3 files modified/created)
- [✓] Architecture: `ShapeRegistry` follows `QueryRegistry` DashMap pattern; `unregister_all_for_connection` uses two-phase collect+remove (safe under DashMap's concurrent guarantees)
- [✓] Non-duplication: Reuses `evaluate_predicate()` from predicate engine; no reimplementation
- [✓] Security: No secrets, no external input without validation, no unsafe blocks
- [✓] Cognitive load: Simple, flat module structure; helper functions in tests reduce repetition

**Summary:** The implementation is functionally complete and correct — all 18 tests pass, all acceptance criteria are met, and the code is well-structured. However, clippy fails with 13 errors (12 doc-comment and `#[must_use]` issues in `shape.rs`, 1 `.map_or` simplification in `shape_evaluator.rs`). The project requires clippy-clean code, making this a critical fix before approval.

### Fix Response v1 (2026-03-21)
**Applied:** All fixes from Review v1

**Fixes:**
1. [✓] Clippy errors (13 total) — all resolved:
   - Added backticks to bare identifiers in doc comments (`shape.rs`: 8 occurrences)
   - Added `#[must_use]` to `unregister()` and `unregister_all_for_connection()` (`shape.rs`)
   - Replaced `.map_or(false, ...)` with `.is_some_and(...)` (`shape_evaluator.rs`)
   - Commit: a7c5558

**Verification:** `cargo clippy --release -p topgun-server -- -D warnings` passes. 582 tests pass, 0 failures.

---

### Review v2 (2026-03-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Fix Verification (from Review v1 CHANGES_REQUESTED):**
- [✓] `.map_or(false, ...)` replaced with `.is_some_and(...)` at `shape_evaluator.rs:41` — confirmed in source
- [✓] `#[must_use]` added to `unregister()` (`shape.rs:80`) and `unregister_all_for_connection()` (`shape.rs:86`) — confirmed in source
- [✓] Doc comment backtick fixes applied throughout `shape.rs` — confirmed in source

**Build Check:** `cargo clippy --release -p topgun-server -- -D warnings` — exits 0, no warnings

**Test Check:** `cargo test --release -p topgun-server` — 582 unit tests pass, 4 integration tests pass, 0 failures

**Shape-specific tests:** All 18 shape tests pass (9 in `shape::tests`, 9 in `shape_evaluator::tests`)

**Passed:**
- [✓] AC1: `shape_evaluator::matches()` evaluates `PredicateNode` filters — EQ, GTE, AND compound all pass
- [✓] AC2: `matches()` returns `true` when filter is `None` — `matches_none_filter_returns_true` passes
- [✓] AC3: `project()` returns only projected fields from a Map value — `project_strips_non_projected_fields` passes
- [✓] AC4: `apply_shape()` combines matching + projection — 3 tests cover all branches
- [✓] AC5: `ShapeRegistry` registers, retrieves, and unregisters correctly — all 8 registry tests pass
- [✓] AC6: `register()` returns `DuplicateShapeId` for duplicate IDs — test verifies exact error variant
- [✓] AC7: `unregister_all_for_connection()` removes all shapes for a connection and returns their IDs
- [✓] AC8: `shapes_for_map()` returns all shapes targeting a specified map (linear scan documented)
- [✓] AC9: All 18 unit tests pass
- [✓] AC10: Full test suite passes (582 server tests + 4 integration tests)
- [✓] R1: Three synchronous free functions — no async in hot evaluation path
- [✓] R2: `ActiveShape` derives `Debug + Clone`; `ShapeRegistryError` derives `thiserror::Error` with `#[error(...)]`; `DashMap<String, ActiveShape>` keyed by `shape_id`; `Default` impl via delegation to `new()`
- [✓] R3: `pub mod shape;` and `pub mod shape_evaluator;` present in `mod.rs` at lines 42-43
- [✓] Constraint: No async — confirmed no `async fn` in either new file
- [✓] Constraint: No DataFusion dependency added
- [✓] Constraint: Max 5 files — 3 files total (2 new, 1 modified)
- [✓] Clippy-clean: `cargo clippy -- -D warnings` exits 0 with all fixes applied
- [✓] Architecture: Follows `QueryRegistry` DashMap pattern; two-phase collect+remove in `unregister_all_for_connection` is correct for DashMap concurrent safety
- [✓] Non-duplication: Delegates to `evaluate_predicate()` from predicate engine; no reimplementation
- [✓] Security: No `unsafe`, no secrets, no external input without validation
- [✓] Cognitive load: Simple flat functions; test helpers (`make_map`, `leaf`, `combinator`, `make_shape`) reduce repetition without adding indirection
- [✓] Rust idioms: `#[must_use]` on all non-trivial return values; `.is_some_and()` used correctly; no unnecessary `.clone()` in production code; no `.unwrap()` in production code

**Summary:** All fixes from Review v1 are correctly applied and verified. The implementation is functionally complete, clippy-clean, and all 18 new unit tests plus the full existing test suite pass. No issues remain.

---

## Completion

**Completed:** 2026-03-21
**Total Commits:** 3 (2 implementation + 1 clippy fix)
**Review Cycles:** 2

### Outcome

Implemented ShapeEvaluator module (matches/project/apply_shape free functions) and ShapeRegistry (DashMap-based concurrent registry) for partial replication shape evaluation and tracking, with 18 unit tests.

### Key Files

- `packages/server-rust/src/service/domain/shape_evaluator.rs` — Pure synchronous functions for evaluating shape filters and field projection against rmpv::Value records
- `packages/server-rust/src/service/domain/shape.rs` — ActiveShape, ShapeRegistry (DashMap-based), ShapeRegistryError for tracking active shapes per connection

### Patterns Established

None — followed existing patterns (QueryRegistry for registry, evaluate_predicate for filter evaluation).

### Deviations

None — implemented as specified.
