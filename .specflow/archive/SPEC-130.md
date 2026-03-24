---
id: SPEC-130
type: feature
status: done
priority: P1
complexity: small
created: 2026-03-19
source: TODO-130
---

# SPEC-130: Derive Arrow Schema from MapSchema

## Context

TopGun's schema system (SPEC-127) defines `MapSchema` with typed `FieldDef` entries using the `FieldType` enum. The upcoming DataFusion SQL layer (TODO-091) requires Apache Arrow `Schema` objects to register tables via `TopGunTableProvider`. This spec bridges the two by converting `FieldType` variants to Arrow `DataType` values.

## Task

Add a `to_arrow_schema` method to `MapSchema` that produces an `arrow_schema::Schema`. Add the `arrow-schema` crate as an optional dependency to `topgun-core` behind an `arrow` feature flag.

## Requirements

### Files to Modify

1. **`packages/core-rust/Cargo.toml`** -- Add `arrow-schema` as an optional dependency under the `arrow` feature flag (minimizes compile footprint; contains exactly `DataType`, `Field`, `Schema`, and `TimeUnit`).

2. **`packages/core-rust/src/schema.rs`** -- Add `impl MapSchema`:
   - `pub fn to_arrow_schema(&self) -> arrow_schema::Schema`

3. **`packages/server-rust/Cargo.toml`** -- Enable the `arrow` feature on the `topgun-core` dependency so the server (where DataFusion runs) gets Arrow schema support.

### Type Mapping

| `FieldType` | Arrow `DataType` | Rationale |
|-------------|-------------------|-----------|
| `String` | `DataType::Utf8` | UTF-8 string |
| `Int` | `DataType::Int64` | Matches `Value::Int(i64)` |
| `Float` | `DataType::Float64` | Matches `Value::Float(f64)` |
| `Bool` | `DataType::Boolean` | Direct mapping |
| `Binary` | `DataType::Binary` | Matches `Value::Bytes` |
| `Timestamp` | `DataType::Timestamp(TimeUnit::Millisecond, None)` | Epoch millis, no timezone |
| `Array(inner)` | `DataType::List(Arc::new(Field::new("item", <inner>, true)))` | Recursive; inner element is nullable |
| `Map` | `DataType::Utf8` | Nested maps are opaque in v1 (no recursive schema). Serialize to JSON string for SQL queryability. |
| `Any` | `DataType::Utf8` | Untyped fields stored as serialized strings for SQL compatibility |

### Nullable Rules

- Fields with `required: true` produce Arrow `Field` with `nullable: false`.
- Fields with `required: false` produce Arrow `Field` with `nullable: true`.

### Method Signature

```rust
impl MapSchema {
    /// Derive an Apache Arrow `Schema` from this map schema.
    ///
    /// Each `FieldDef` becomes an Arrow `Field` with the corresponding
    /// `DataType`. The `required` flag maps to Arrow nullability (required
    /// fields are non-nullable).
    #[must_use]
    pub fn to_arrow_schema(&self) -> arrow_schema::Schema {
        // ...
    }
}
```

A private helper `fn field_type_to_arrow(ft: &FieldType) -> DataType` handles the recursive `Array` case.

## Acceptance Criteria

1. `MapSchema::to_arrow_schema()` returns an `arrow_schema::Schema` with one `Field` per `FieldDef`.
2. Each of the 9 `FieldType` variants maps to the Arrow `DataType` listed in the table above.
3. `required: true` fields produce `nullable: false` Arrow fields; `required: false` produce `nullable: true`.
4. Nested `FieldType::Array(Box<FieldType::Int>)` produces `DataType::List(Field("item", Int64, true))`.
5. All existing tests continue to pass (no regressions from adding the `arrow-schema` dependency).
6. At least 9 unit tests (one per `FieldType` variant) plus 1 test for nullable/non-nullable mapping, 1 test for nested arrays, and 1 test for a complete multi-field schema round-trip.

## Constraints

- Do NOT add `arrow-schema` as a required dependency -- use a Cargo feature flag `arrow` that is off by default, so downstream consumers that do not need Arrow are not burdened with the compile cost.
- Do NOT convert `Value` instances to Arrow arrays in this spec. Only schema (type metadata) conversion is in scope.
- Do NOT attempt recursive struct conversion for `FieldType::Map`. The `Map` variant has no inner schema in v1; treat it as opaque `Utf8`.
- The method is infallible (`-> Schema`, not `-> Result<Schema>`). All `FieldType` variants have a defined mapping.

## Assumptions

- The `arrow-schema` crate version will be the latest stable (v54+). It is a lighter sub-crate that provides exactly `DataType`, `Field`, `Schema`, and `TimeUnit` without pulling in compute, IPC, or other Arrow modules.
- `FieldType::Map` and `FieldType::Any` map to `Utf8` because DataFusion can still query JSON-encoded strings via `json_` functions. This is a pragmatic v1 choice; future specs may introduce `DataType::Struct` when `Map` gains inner field definitions.
- The feature flag will be named `arrow` on `topgun-core`. The `to_arrow_schema` method will be gated behind `#[cfg(feature = "arrow")]`.
- `server-rust/Cargo.toml` will enable the `arrow` feature on `topgun-core` since the server is where DataFusion runs.

## Audit History

### Audit v1 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~10% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~10% | <=50% | OK |
| Largest task group | ~10% | <=30% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <-- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions:**

| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Clear task, rationale, exact type mapping table |
| Completeness | Pass | All 9 variants mapped, nullable rules defined, method signature provided |
| Testability | Pass | AC6 specifies exact test count and coverage per variant |
| Scope | Pass | Explicit "do NOT" constraints prevent scope creep |
| Feasibility | Pass | Simple type mapping, arrow-schema is lightweight |
| Architecture fit | Pass | Follows existing pattern: pure functions in core-rust, feature-gated |
| Non-duplication | Pass | No Arrow integration exists in codebase |
| Cognitive load | Pass | Single mapping function with helper, trivial logic |
| Strategic fit | Pass | Direct prerequisite for DataFusion SQL layer (TODO-091), v2.0 roadmap |
| Project compliance | Pass | Rust, feature-gated optional dep, follows conventions |

**Language Profile:** OK -- 3 files (2 Cargo.toml + 1 schema.rs), well under limit of 5. Small complexity, trait-first not applicable.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | arrow-schema sub-crate provides all needed types | Would need broader arrow features, slightly larger compile |
| A2 | Map/Any as Utf8 is sufficient for v1 SQL | DataFusion queries on nested data would need json_ functions |
| A3 | arrow v54+ is latest stable | Version number may differ, no functional impact |

**Project compliance:** Honors PROJECT.md decisions -- feature-gated dependency, Rust patterns, no new required deps.

**Strategic fit:** Aligned with project goals -- bridges schema system to DataFusion SQL layer.

**Comment:** Well-structured specification with clear type mapping, proper feature gating, and well-scoped boundaries. Ready for implementation.

**Recommendations:**
1. Consider depending on `arrow-schema` crate directly (instead of `arrow` with `datatypes` feature) for an even smaller compile footprint. The `arrow-schema` crate contains exactly `DataType`, `Field`, `Schema`, and `TimeUnit` -- all that is needed here.
2. The "Files to Modify" section should explicitly list `packages/server-rust/Cargo.toml` as a third file to modify (to enable the `arrow` feature on `topgun-core`), since this is mentioned in Assumptions but not in the file list.

### Response v1 (2026-03-19)
**Applied:** All recommendations (R1, R2)

**Changes:**
1. [✓] Use `arrow-schema` crate directly — Updated Task section, Files to Modify item 1, Constraints, Assumptions, and method return type to reference `arrow-schema` crate instead of `arrow` with `datatypes` feature. Lighter compile footprint; provides exactly the needed types.
2. [✓] Add `packages/server-rust/Cargo.toml` to Files to Modify — Added as item 3 with explicit description (enable `arrow` feature on `topgun-core` dependency). Previously only mentioned in Assumptions.

---

## Execution Summary

**Executed:** 2026-03-19
**Commits:** 2

### Files Created
None.

### Files Modified
- `packages/core-rust/Cargo.toml` — Added `[features]` section with `arrow = ["dep:arrow-schema"]` and `arrow-schema = { version = "55", optional = true }` dependency
- `packages/server-rust/Cargo.toml` — Enabled `features = ["arrow"]` on `topgun-core` dependency
- `packages/core-rust/src/schema.rs` — Added `MapSchema::to_arrow_schema()` method, `field_type_to_arrow()` private helper, and 13 unit tests in `schema::tests::arrow_tests` module

### Files Deleted
None.

### Acceptance Criteria Status
- [x] `MapSchema::to_arrow_schema()` returns an `arrow_schema::Schema` with one `Field` per `FieldDef`
- [x] Each of the 9 `FieldType` variants maps to the Arrow `DataType` listed in the spec table
- [x] `required: true` fields produce `nullable: false`; `required: false` produce `nullable: true`
- [x] `FieldType::Array(Box<FieldType::Int>)` produces `DataType::List(Field("item", Int64, true))`
- [x] All 467 existing core tests + 559 server tests continue to pass (no regressions)
- [x] 13 unit tests: 9 per FieldType variant (String, Int, Float, Bool, Binary, Timestamp, Array, Map, Any) + 2 nullable/non-nullable + 1 nested array + 1 multi-field round-trip

### Deviations
1. [Rule 1 - Bug] Clippy `match_same_arms` error: `FieldType::String`, `FieldType::Map`, and `FieldType::Any` all map to `DataType::Utf8`. Merged into a single match arm `FieldType::String | FieldType::Map | FieldType::Any => DataType::Utf8` to satisfy `-D warnings`. Functionally identical to the spec table.

### Notes
- Used `arrow-schema` v55 (latest stable at time of execution; spec noted v54+, A3 assumption was correct that version might differ but no functional impact).
- `String`, `Map`, and `Any` are merged into one match arm per Clippy's `match_same_arms` lint. The type mapping is identical to the spec.
- All feature-gated code is wrapped in `#[cfg(feature = "arrow")]` so the `default` build of `topgun-core` (without the flag) has zero Arrow compile cost.

### Audit v2 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~10% total

**Dimensions:** All 10 dimensions pass (unchanged from v1). Revisions correctly addressed both prior recommendations:
- R1: `arrow-schema` crate is now specified directly throughout the spec (Files to Modify, Constraints, Assumptions).
- R2: `packages/server-rust/Cargo.toml` is now listed as item 3 in Files to Modify.

**Rust Auditor Checklist:** Not applicable -- this spec adds no message structs, no new serde-annotated types, and no integer-semantic fields. The implementation adds a pure mapping function to an existing struct.

**Language Profile:** OK -- 3 files, well under limit of 5. Small complexity, trait-first not applicable.

**Project compliance:** Honors PROJECT.md decisions -- feature-gated optional dependency, Rust patterns, no new required deps.

**Strategic fit:** Aligned with project goals -- direct prerequisite for DataFusion SQL layer (TODO-091), v2.0 roadmap.

**Comment:** Both prior recommendations have been cleanly incorporated. Spec is complete, clear, and ready for implementation. No further issues identified.

---

## Review History

### Review v1 (2026-03-19)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `to_arrow_schema()` returns `arrow_schema::Schema` with one `Field` per `FieldDef` — confirmed by `multi_field_schema_round_trip` test asserting `arrow.fields().len() == 9`
- [✓] AC2: All 9 `FieldType` variants mapped correctly — `String`/`Map`/`Any` to `Utf8`, `Int` to `Int64`, `Float` to `Float64`, `Bool` to `Boolean`, `Binary` to `Binary`, `Timestamp` to `Timestamp(Millisecond, None)`, `Array(inner)` to `List`; 9 individual variant tests confirm each mapping
- [✓] AC3: Nullable rules correct — `nullable = !fd.required` inverts the required flag; `required_true_produces_non_nullable_field` and `required_false_produces_nullable_field` tests cover both cases
- [✓] AC4: `Array(Box<Int>)` produces `List(Field("item", Int64, true))` — `array_of_int_maps_to_list_int64` test verifies inner type and item nullability
- [✓] AC5: No regressions — core-rust: 454 tests pass without arrow feature, 467 with (13 new tests); server: 559 tests pass unchanged
- [✓] AC6: 13 unit tests in `schema::tests::arrow_tests` module — exceeds the minimum of 12 (9 per variant + 2 nullable + 1 nested array + 1 round-trip)
- [✓] Feature flag constraint: `arrow` feature off by default; all Arrow code gated behind `#[cfg(feature = "arrow")]`; default build (without flag) compiles to 454 passing tests with zero Arrow compile cost
- [✓] Infallible method: returns `arrow_schema::Schema` not `Result<Schema, _>`; `#[must_use]` attribute present
- [✓] Private helper `field_type_to_arrow()` correctly handles recursive `Array` case
- [✓] `server-rust/Cargo.toml`: `topgun-core` dependency has `features = ["arrow"]` enabling Arrow support where DataFusion runs
- [✓] Build check: clippy passes with `-D warnings` on both the arrow feature build and default build
- [✓] No hardcoded secrets, no unsafe blocks, no unwrap() in production code
- [✓] Deviation (merged match arms) correctly documented and functionally equivalent — Clippy `match_same_arms` lint requires merging `String | Map | Any => Utf8` into one arm; mapping is identical to spec
- [✓] Code style consistent with existing `schema.rs` patterns (section headers, doc comments, test organization)
- [✓] No code duplication — no existing Arrow utilities reinvented

**Summary:** All 6 acceptance criteria are met exactly. The implementation is clean, minimal, and correct. Feature gating works at both levels (default build excludes Arrow entirely; server build includes it). 13 tests pass against a clippy-clean codebase. No issues found.

---

## Completion

**Completed:** 2026-03-19
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Added `MapSchema::to_arrow_schema()` to core-rust behind an optional `arrow` feature flag, bridging TopGun's schema system to Apache Arrow for the upcoming DataFusion SQL layer. All 9 FieldType variants mapped with correct nullability rules.

### Key Files

- `packages/core-rust/src/schema.rs` — `to_arrow_schema()` method and `field_type_to_arrow()` helper with 13 unit tests

### Patterns Established

None — followed existing patterns (feature-gated optional dependency, pure functions in core-rust).

### Deviations

1. Merged `String | Map | Any` match arms into one per Clippy `match_same_arms` lint. Functionally identical to spec table.
