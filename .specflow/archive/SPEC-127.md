---
id: SPEC-127
type: feature
status: done
priority: P1
complexity: medium
created: 2026-03-19
source: TODO-069
---

# SPEC-127: Schema Types, Validation Engine, and SchemaService

## Context

TODO-069 (Schema System) is the foundational data model for v2.0. It spans TS codegen tooling, Rust schema types, server validation, and Arrow type mapping. This spec covers the **first slice**: expanding the placeholder `MapSchema`/`FieldDef` types in `core-rust`, building a validation engine that checks `Value` against typed field definitions, and implementing `SchemaService` (the `SchemaProvider` trait).

The existing code has:
- Placeholder `MapSchema` with only `version: u32` and `fields: Vec<FieldDef>` (no type info)
- Placeholder `FieldDef` with only `name: String` and `required: bool` (no type constraints)
- `ValidationResult` enum (Valid/Invalid with error strings)
- `SchemaProvider` trait in `server-rust/src/traits.rs` with `get_schema`, `register_schema`, `validate`, `get_shape`
- `CrdtService` already has `WriteValidator` for permission/size checks but no schema validation

Subsequent specs (not in scope here) will cover:
- TS schema definition DSL and `topgun.schema.ts` codegen toolchain
- CrdtService wiring (calling SchemaProvider::validate in the write path)
- Arrow column type derivation for DataFusion (TODO-091)
- SyncShape computation for partial replication (TODO-070)

## Goal Analysis

**Goal Statement:** Server can validate incoming CRDT writes against typed field definitions, rejecting invalid data before it enters the merge path.

**Observable Truths:**
1. A `MapSchema` carries typed field definitions (string, int, float, bool, array, map, timestamp, binary) with constraints (required, min/max, regex, enum values)
2. `SchemaService` implements `SchemaProvider` trait, storing schemas in a concurrent map
3. `validate()` returns `Valid` or `Invalid { errors }` by checking each field's type and constraints against a `Value::Map`
4. Schemas can be registered and retrieved by map name at runtime
5. Unregistered maps pass validation (optional-mode: no schema = no validation)

**Required Artifacts:**
- `packages/core-rust/src/schema.rs` — expanded types (FieldType, FieldConstraint, MapSchema, FieldDef) and validate_value/validate_schema functions
- `packages/core-rust/src/lib.rs` — re-export new public types (FieldType, FieldConstraint)
- `packages/core-rust/Cargo.toml` — add `regex` dependency
- `packages/server-rust/src/service/domain/schema.rs` — SchemaService implementing SchemaProvider
- `packages/server-rust/src/service/domain/mod.rs` — add schema module declaration

**Key Links:**
- `SchemaService::validate()` depends on `FieldType` + `FieldConstraint` from core-rust
- `SchemaProvider` trait (already defined) is the contract SchemaService implements
- Future CrdtService wiring will call `SchemaProvider::validate()` in the write path (out of scope)

## Task

1. Expand `MapSchema` and `FieldDef` in `core-rust/src/schema.rs` with a `FieldType` enum and `FieldConstraint` struct
2. Implement a `validate_value` function that checks a `Value` against a `MapSchema`, compiling regex patterns on each call
3. Implement a `validate_schema` function for registration-time pattern validation (compiles all regexes, returns errors for invalid patterns)
4. Create `SchemaService` in `server-rust` that implements the existing `SchemaProvider` trait; `register_schema` calls `validate_schema` before inserting
5. Update `core-rust/src/lib.rs` to re-export `FieldType` and `FieldConstraint`
6. Add unit tests for validation logic and SchemaService

## Requirements

### File: `packages/core-rust/src/schema.rs` (modify)

Expand the existing placeholder types:

**`FieldType` enum** (new):
- `String` — matches `Value::String`
- `Int` — matches `Value::Int`
- `Float` — matches `Value::Float` or `Value::Int` (int-to-float widening coercion)
- `Bool` — matches `Value::Bool`
- `Binary` — matches `Value::Bytes`
- `Timestamp` — matches `Value::Int` (epoch millis, i64)
- `Array(Box<FieldType>)` — matches `Value::Array`, element type checked recursively
- `Map` — matches `Value::Map` (nested map, no recursive schema validation in v1)
- `Any` — matches any non-Null `Value` variant

Derive: `Debug, Clone, Default, PartialEq, Serialize, Deserialize`. The `Any` variant is the default (annotate with `#[default]`), representing an untyped field for backward compatibility with pre-schema data.

Do NOT add `#[serde(rename_all = "camelCase")]` to `FieldType` — PascalCase variant names are acceptable for this internal/server-side type, and applying camelCase would produce lowercase variant tags which are less readable.

**`FieldConstraint` struct** (new):
- `min_length: Option<u32>` — for String: min UTF-8 char count; for Array: min element count
- `max_length: Option<u32>` — for String: max UTF-8 char count; for Array: max element count
- `min_value: Option<i64>` — for Int/Timestamp: inclusive minimum. Not applicable to Float fields (see Known Limitations)
- `max_value: Option<i64>` — for Int/Timestamp: inclusive maximum. Not applicable to Float fields (see Known Limitations)
- `pattern: Option<String>` — for String: regex pattern string; compiled by `validate_value` on each call and by `validate_schema` at registration time
- `enum_values: Option<Vec<String>>` — for String: allowed values whitelist

Derive: `Debug, Clone, Default, PartialEq, Serialize, Deserialize`. Add `#[serde(rename_all = "camelCase")]`. All fields `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none", default)]`.

**`FieldDef` struct** (modify existing):
- `name: String` (keep)
- `required: bool` (keep)
- `field_type: FieldType` (add) — annotate with `#[serde(default)]` for backward compatibility; defaults to `FieldType::Any`
- `constraints: Option<FieldConstraint>` (add) — annotate with `#[serde(skip_serializing_if = "Option::is_none", default)]`

Do NOT add `#[serde(rename_all = "camelCase")]` to `FieldDef` — the existing serialized fields use snake_case (`name`, `required`). Adding camelCase now would break wire compatibility with any existing serialized schemas. The new fields `field_type` and `constraints` follow snake_case by default in serde, which is consistent.

**`MapSchema` struct** (modify existing):
- `version: u32` (keep)
- `fields: Vec<FieldDef>` (keep)
- `strict: bool` (add) — if true, reject records with fields not defined in schema; if false, extra fields are allowed. Annotate with `#[serde(default)]` for backward compatibility (defaults to `false`)

Do NOT add `#[serde(rename_all = "camelCase")]` to `MapSchema` — same wire compatibility reason as `FieldDef` above.

**`validate_value` function** (new, pub):
- Signature: `pub fn validate_value(schema: &MapSchema, value: &Value) -> ValidationResult`
- If `value` is not `Value::Map`, return `Invalid` with error "expected a Map value"
- For each `FieldDef` where `required == true`: check field exists and is not `Value::Null`; if missing or null, add error "field '<name>' is required"
- For each field present in the value's map: if a matching `FieldDef` exists, check type compatibility and constraints
- If `schema.strict == true` and value contains fields not in schema, add error "unknown field '<name>'" for each unknown field
- Return `Valid` if zero errors, `Invalid { errors }` otherwise
- When a `pattern` constraint is present, compile it via `regex::Regex::new()` on each call; if the pattern is malformed, add error "field '<name>': invalid pattern '<pattern>'"

**`validate_schema` function** (new, pub):
- Signature: `pub fn validate_schema(schema: &MapSchema) -> Result<(), Vec<String>>`
- For each `FieldDef` that has a `constraints.pattern`: attempt `regex::Regex::new(pattern)`; collect all failures
- Return `Ok(())` if all patterns compile, `Err(errors)` otherwise
- Purpose: called by `SchemaService::register_schema` before inserting to catch invalid patterns early

**Type checking rules:**
- `FieldType::String` accepts `Value::String`
- `FieldType::Int` accepts `Value::Int`
- `FieldType::Float` accepts `Value::Float` or `Value::Int` (int-to-float widening coercion; JS clients often send integers where floats are expected)
- `FieldType::Bool` accepts `Value::Bool`
- `FieldType::Binary` accepts `Value::Bytes`
- `FieldType::Timestamp` accepts `Value::Int`
- `FieldType::Array(inner)` accepts `Value::Array`, checks each element against `inner`
- `FieldType::Map` accepts `Value::Map`
- `FieldType::Any` accepts any variant except `Value::Null`

**Constraint checking rules:**
- `min_length`/`max_length` on String: check `value.chars().count()`
- `min_length`/`max_length` on Array: check `vec.len()`
- `min_value`/`max_value` on Int/Timestamp: compare i64 value
- `pattern` on String: compile `regex::Regex::new(pattern)` per call, match against full string value
- `enum_values` on String: check value is in the whitelist

### File: `packages/core-rust/src/lib.rs` (modify)

Extend the existing `pub use schema::` re-export line to include the two new public types:

```
pub use schema::{FieldConstraint, FieldDef, FieldType, MapSchema, Predicate, SyncShape, ValidationResult, validate_schema, validate_value};
```

### File: `packages/server-rust/src/service/domain/schema.rs` (create)

**`SchemaService` struct**:
- Fields: `schemas: DashMap<String, MapSchema>` (concurrent map, keyed by map name)
- Implements `SchemaProvider` trait:
  - `get_schema`: lookup in DashMap, clone and return
  - `register_schema`: call `validate_schema(schema)` first; if `Err(errors)`, return an error (or log and skip, depending on how `SchemaProvider::register_schema` is typed); if `Ok`, insert into DashMap, overwriting existing
  - `validate`: lookup schema; if None, return `ValidationResult::Valid` (optional mode); if Some, call `validate_value`
  - `get_shape`: return `None` (stub for this spec; shape computation is TODO-070). Note: `SchemaProvider::get_shape` takes `&RequestContext` — import `topgun_core::RequestContext` alongside other core imports
- Implements `ManagedService` trait: `name()` returns `"schema"`; `init()`, `reset()`, and `shutdown()` are no-ops returning `Ok(())`
- Constructor: `SchemaService::new() -> Self`

### File: `packages/server-rust/src/service/domain/mod.rs` (modify)

- Add `pub mod schema;` declaration
- Re-export `SchemaService`

### File: `packages/core-rust/Cargo.toml` (modify)

- Add `regex` crate dependency (for pattern constraint validation in `validate_value` and `validate_schema`)

## Acceptance Criteria

1. `FieldType` enum has all 9 variants listed above, with `Debug, Clone, Default, PartialEq, Serialize, Deserialize`; `Any` is the `#[default]` variant
2. `FieldConstraint` has all 6 optional fields, derives `Default`, has `#[serde(rename_all = "camelCase")]`
3. `FieldDef` has `field_type: FieldType` with `#[serde(default)]` and `constraints: Option<FieldConstraint>` with `#[serde(skip_serializing_if = "Option::is_none", default)]`
4. `MapSchema` has `strict: bool` with `#[serde(default)]`
5. `validate_value` correctly validates a `Value::Map` against a `MapSchema`:
   - Returns `Valid` for conforming data
   - Returns `Invalid` with specific error messages for: missing required field, wrong type, constraint violation, unknown field (strict mode)
6. `validate_value` returns `Invalid` when value is not a `Value::Map`
7. `FieldType::Binary` matches `Value::Bytes` (not `Value::Binary`, which does not exist)
8. `validate_schema` returns `Ok(())` when all patterns are valid regexes; returns `Err(errors)` listing invalid patterns
9. `SchemaService::register_schema` calls `validate_schema` before inserting; rejects schemas with invalid regex patterns
10. `SchemaService` implements `SchemaProvider`: register (with pattern validation), retrieve, validate
11. `SchemaService::validate` returns `Valid` when no schema is registered for the map (optional mode)
12. `core-rust/src/lib.rs` re-exports `FieldType`, `FieldConstraint`, `validate_value`, and `validate_schema`
13. All existing tests continue to pass (no breaking changes to `MapSchema` serialization — new fields use `#[serde(default)]`)

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-core` — all pass
2. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` — all pass
3. Create a `MapSchema` with `strict: true`, a required String field with `max_length: 10`, and validate a `Value::Map` with that field set to an 11-char string — returns `Invalid` with constraint error
4. Create a `MapSchema` with a field having `pattern: "[invalid"` — `validate_schema` returns `Err` with pattern error; `SchemaService::register_schema` rejects it
5. Serialize a `MapSchema` with new fields via `rmp_serde::to_vec_named`, deserialize it back — round-trips correctly
6. Deserialize an old-format `MapSchema` (without `field_type`, `constraints`, `strict`) — succeeds with defaults (backward compatible)
7. Validate a `Value::Map` containing a `Value::Bytes` field against a schema with `FieldType::Binary` — returns `Valid`

## Constraints

- Do NOT wire SchemaService into CrdtService's write path (that is a separate spec)
- Do NOT implement Arrow type derivation (that is for the DataFusion spec, TODO-091)
- Do NOT implement SyncShape computation logic (that is for TODO-070)
- Do NOT create the TS schema DSL or codegen toolchain (separate spec)
- Do NOT add `schema` as a CLI flag or config option (runtime registration only for now)
- Keep `SchemaProvider::get_shape` returning `None` (stub)
- Max 5 files modified (Rust language profile constraint)
- Do NOT add `#[serde(rename_all = "camelCase")]` to `MapSchema` or `FieldDef` — these existing structs use snake_case wire format and adding camelCase now would be a breaking change

## Assumptions

- `regex` crate is acceptable as a new dependency for core-rust (lightweight, widely used)
- `FieldType::Float` accepting `Value::Int` (widening coercion) is the correct behavior — JS clients often send integers where floats are expected
- `FieldType::Timestamp` is represented as `Value::Int` (epoch millis) rather than a dedicated Value variant — consistent with existing `Timestamp.millis` convention
- Nested map validation (recursive MapSchema for `FieldType::Map`) is deferred — v1 only checks that the value is a Map, not its contents
- `DashMap` (already a dependency) is appropriate for schema storage since schemas are registered infrequently but read on every write
- Adding `#[serde(default)]` on new fields in `MapSchema` and `FieldDef` provides backward compatibility without breaking existing serialized data
- **Regex compilation strategy:** `validate_value` compiles regex patterns on every call (simple, no extra state on `MapSchema`). `validate_schema` provides registration-time validation via `SchemaService::register_schema` to catch invalid patterns early. This is a deliberate tradeoff: `validate_value` is a pure function with no regex cache, keeping `MapSchema` serializable without a separate compiled form.

## Known Limitations

- `FieldConstraint.min_value`/`max_value` are `Option<i64>` and cannot constrain `FieldType::Float` fields. Float range validation (if needed in a future spec) would require adding `min_float_value: Option<f64>` / `max_float_value: Option<f64>` to `FieldConstraint`.

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Expand schema types in core-rust: FieldType enum (with Default), FieldConstraint struct (with camelCase), modify FieldDef and MapSchema with serde defaults; add validate_schema fn; update lib.rs re-exports; add regex dep | — | ~12% |
| G2 | 2 | Implement validate_value in core-rust (type checking, constraints, regex compilation per call); create SchemaService in server-rust (SchemaProvider impl, register calls validate_schema, ManagedService); update domain/mod.rs | G1 | ~22% |
| G3 | 3 | Unit tests for validate_value (core-rust): type checks, constraints, strict mode, Binary->Bytes, backward compat deserialization; unit tests for SchemaService: register/retrieve/validate, invalid pattern rejection, optional mode | G2 | ~12% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2 | No | 1 |
| 3 | G3 | No | 1 |

**Total workers needed:** 1 (sequential — each group depends on the previous)

## Audit History

### Audit v1 (2026-03-19)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (sum of groups: 25+30+20+25)

**Critical:**
1. **Value::Binary does not exist** — The spec references `Value::Binary` in FieldType::Binary matching rules and type checking rules, but the actual `Value` enum in `core-rust/src/types.rs` uses `Bytes(Vec<u8>)`, not `Binary`. All references to `Value::Binary` must be changed to `Value::Bytes`. The `FieldType::Binary` variant name is fine, but the matching target is wrong.

2. **Missing `core-rust/src/lib.rs` modification** — The spec adds `FieldType` and `FieldConstraint` as public types in `schema.rs` but does not list `packages/core-rust/src/lib.rs` as a file to modify. Server-rust imports all core types via `topgun_core::` root re-exports (line: `pub use schema::{FieldDef, MapSchema, Predicate, SyncShape, ValidationResult};`). Without adding `FieldType` and `FieldConstraint` to this re-export, `SchemaService` in server-rust cannot access them. This is a 5th file (within the 5-file limit) and must be specified.

3. **Regex caching architecture is contradictory** — Acceptance criterion #9 requires regex compilation at registration time (not validation time). The Assumptions section says "SchemaService stores compiled regexes alongside the schema." However, `validate_value` is a standalone `pub fn` in `core-rust` that receives `&MapSchema` — and `MapSchema` stores `pattern: Option<String>` (not compiled `Regex`, which is not Serialize/Deserialize). So `validate_value` must compile regex on every call, contradicting both the criterion and assumption. Resolution options: (a) `validate_value` compiles regex each call (simple but contradicts stated goal), (b) add a `CompiledSchema` wrapper in `SchemaService` that holds `HashMap<String, Regex>` alongside `MapSchema` and pass compiled regexes to a separate internal validation function, or (c) `validate_value` accepts an optional `&HashMap<String, Regex>` parameter. The spec must pick one and specify it clearly.

4. **`FieldDef.field_type` default not specified** — The spec says to add `#[serde(default)]` on new fields for backward compatibility. `FieldDef.field_type: FieldType` requires `FieldType` to implement `Default`, but the spec does not include `Default` in the derive list for `FieldType` and does not specify which variant is the default. This must be explicit (likely `FieldType::Any` for backward compat — "untyped field accepts anything"). Add `Default` to `FieldType` derives and specify `#[default]` on the `Any` variant.

5. **Missing `#[serde(rename_all = "camelCase")]` decision** — PROJECT.md auditor checklist requires `#[serde(rename_all = "camelCase")]` on every struct. The existing `MapSchema`, `FieldDef`, `ValidationResult` do NOT have it (verified in source). Adding it to existing structs would break wire compatibility. The spec must explicitly state: (a) new types (`FieldType`, `FieldConstraint`) get `rename_all = "camelCase"`, and (b) existing types (`MapSchema`, `FieldDef`) intentionally omit it to preserve backward compatibility, OR (c) all types get it now (accepting the breaking change). The current silence on this is ambiguous.

**Recommendations:**
6. [Strategic] The estimated context across groups sums to 100%, which far exceeds the 50% target. However, since the groups are split across waves with max 2 workers, per-group estimates are the real concern. G2 at ~30% is at the warning threshold. Consider whether G4 (tests) could be merged into G2/G3 (write tests alongside implementation) to reduce waves from 3 to 2, since tests are typically co-located in Rust `#[cfg(test)]` modules.

7. `FieldConstraint.min_value`/`max_value` are `Option<i64>` which cannot constrain `Float` values. If a future `FieldType::Float` field needs min/max constraints, this will require a separate `min_float_value: Option<f64>` or a redesign. Consider noting this as a known limitation or adding `min_float_value`/`max_float_value` now.

8. The `pattern` constraint is specified to compile regex at validation time in `validate_value` (since it only has the string), but the spec also wants registration-time validation. Consider adding a `pub fn validate_schema(schema: &MapSchema) -> Result<(), Vec<String>>` function in core-rust that checks all patterns are valid regexes. `SchemaService::register_schema` would call this before inserting.

### Response v1 (2026-03-19)
**Applied:** all (5 critical + 3 recommendations)

**Changes:**
1. [✓] Value::Binary -> Value::Bytes — Updated all occurrences in type checking rules, FieldType::Binary description, and added Acceptance Criterion #7 explicitly testing `Value::Bytes` matching
2. [✓] Missing lib.rs modification — Added `packages/core-rust/src/lib.rs` to Required Artifacts and Requirements with exact updated re-export line including `FieldType`, `FieldConstraint`, `validate_value`, `validate_schema`
3. [✓] Regex caching contradiction — Resolved by choosing option (a): `validate_value` compiles regex per call (simple, keeps `MapSchema` serializable); `validate_schema` provides registration-time validation called by `SchemaService::register_schema`. Both the old contradictory criterion and assumption replaced with clear statements. Acceptance criterion #9 updated to describe `validate_schema` + `register_schema` flow; former criterion about "compiled at registration" removed.
4. [✓] FieldType Default not specified — Added `Default` to `FieldType` derive list, specified `#[default]` on `Any` variant with rationale ("untyped field accepts anything, backward compat")
5. [✓] serde rename_all camelCase decision — Made explicit: `FieldConstraint` (new struct) gets `rename_all = "camelCase"`; `FieldType` (enum, not applicable); `MapSchema` and `FieldDef` (existing structs) intentionally omit it with documented wire-compat rationale. Added to Constraints section.
6. [✓] Merge G4 tests into fewer waves — Reorganized from 4 groups (3 waves) to 3 groups (3 waves) by consolidating G2+G3 implementation work and creating G3 as a dedicated test group. G2 now handles both SchemaService creation and validate_value implementation in a single wave, reducing total waves. Removed 4-worker notation; sequential execution now explicit.
7. [✓] Float min/max limitation — Added "Known Limitations" section documenting that `min_value`/`max_value` are `Option<i64>` and do not apply to Float fields, with note about future path.
8. [✓] validate_schema function — Added `validate_schema` as a required public function in core-rust, included in Task #3 and #4, added to Required Artifacts, Requirements, and Acceptance Criteria. `SchemaService::register_schema` calls it before inserting.

### Audit v2 (2026-03-19)
**Status:** NEEDS_REVISION

**Context Estimate:** ~45% total (independent estimate: G1 ~12%, G2 ~22%, G3 ~12%, overhead ~5%)

Note: The spec's own group estimates (30+45+25 = 100%) are significantly inflated. Type definitions, one validation function, a thin service wrapper, and tests for a 5-file change do not consume 100% context. Independent estimates based on file types and complexity multipliers yield ~45% total, which is within the 50% target.

**Critical:**
1. **ManagedService method names are wrong** -- The spec says "Implements `ManagedService` trait (start/stop as no-ops; name returns `"schema"`)" but the actual `ManagedService` trait (in `server-rust/src/service/registry.rs`) defines `name()`, `init(&self, ctx: &ServiceContext) -> anyhow::Result<()>`, `reset(&self) -> anyhow::Result<()>`, and `shutdown(&self, terminate: bool) -> anyhow::Result<()>`. There are no `start`/`stop` methods. The spec must use the correct method names: `init`, `reset`, and `shutdown` as no-ops returning `Ok(())`.

**Recommendations:**
2. **FieldType serde rename_all reasoning is technically incorrect** -- The spec states `#[serde(rename_all = "camelCase")]` "would have no effect on variant names" for enums. This is wrong -- `rename_all` on an enum DOES rename variant tags (e.g., `Array` would become `array`). The conclusion to omit it is still defensible (PascalCase variants are fine for an internal type), but the stated reasoning should be corrected to: "Do NOT add `#[serde(rename_all = "camelCase")]` to `FieldType` -- PascalCase variant names are acceptable for this internal/server-side type, and applying camelCase would produce lowercase variant tags which are less readable."

3. **G2 context estimate of ~45% is inflated** -- Independent estimate puts G2 at ~22% (validate_value ~8% with 1.3x business logic multiplier, SchemaService creation ~5%, mod.rs ~1%, worker overhead ~5%, file reads ~3%). Consider updating estimates to: G1 ~12%, G2 ~22%, G3 ~12%.

4. **SchemaService imports note** -- The `SchemaProvider::get_shape` method takes `&RequestContext` as a parameter. Since `SchemaService` just returns `None`, this is trivial, but the implementor needs to know that `topgun_core::RequestContext` must be imported. Consider mentioning this in the file requirements for completeness.

**Dimension Summary:**
- Clarity: Good -- requirements are specific with exact type signatures and serde annotations
- Completeness: Good -- all 5 files listed, constraints and assumptions documented
- Testability: Good -- 13 acceptance criteria, all measurable; 7-item validation checklist
- Scope: Good -- clear boundaries with explicit "do NOT" constraints
- Feasibility: Good -- straightforward Rust implementation
- Architecture fit: Good -- follows existing patterns (DashMap, ManagedService, SchemaProvider trait)
- Non-duplication: Good -- implements existing trait, no reinvention
- Cognitive load: Good -- simple types + one validation function + thin service wrapper
- Strategic fit: Aligned with project goals (v2.0 schema system foundation)
- Project compliance: Honors PROJECT.md decisions; wire compat preserved; 5-file limit respected
- Language profile: Compliant with Rust profile (5 files, trait-first with types in G1)

### Response v2 (2026-03-19)
**Applied:** all (1 critical + 3 recommendations)

**Changes:**
1. [✓] ManagedService method names corrected — Changed "start/stop as no-ops; name returns `"schema"`" to "`name()` returns `"schema"`; `init()`, `reset()`, and `shutdown()` are no-ops returning `Ok(())`" in the SchemaService struct description
2. [✓] FieldType serde rename_all reasoning corrected — Replaced technically incorrect statement ("would have no effect on variant names") with accurate reasoning: "PascalCase variant names are acceptable for this internal/server-side type, and applying camelCase would produce lowercase variant tags which are less readable"
3. [✓] Task group context estimates updated — G1 ~30% → ~12%, G2 ~45% → ~22%, G3 ~25% → ~12% in the Task Groups table
4. [✓] SchemaService imports note added — Added note in the `get_shape` bullet: "import `topgun_core::RequestContext` alongside other core imports"

### Audit v3 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~51% total (G1 ~12%, G2 ~22%, G3 ~12%, overhead ~5%)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~51% | ≤50% | At boundary |
| Largest task group | ~22% | ≤30% | OK |
| Worker overhead | ~5% | ≤10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | |
| 70%+ | POOR | |

**Assumptions validated:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | `regex` crate acceptable for core-rust | Would need alternative pattern matching; low risk (regex is standard) |
| A2 | Int-to-float widening coercion is correct | JS clients might fail validation; easily reversible per-field |
| A3 | Nested map validation deferred to v2 | Deep structures pass without content checks; acceptable for first slice |
| A4 | DashMap suitable for schema storage | Write-rare/read-often pattern matches DashMap well; no concern |
| A5 | Regex compilation per call is acceptable | Hot path performance risk if many regex fields; mitigated by validate_schema at registration |

**Project compliance:** Honors PROJECT.md decisions (MsgPack wire format, no new runtime deps for server-rust, proper integer types, Default on multi-optional structs, enum for FieldType)

**Strategic fit:** Aligned with project goals -- v2.0 schema system foundation

**Language profile:** Compliant with Rust profile (5 files, trait-first with types in G1, no group exceeds 3 files)

**Goal-backward validation:** All 5 observable truths have corresponding artifacts. All key links identified and connected. No orphan artifacts.

**Comment:** Well-structured spec after two revision cycles. All prior critical issues resolved. Requirements are precise with exact type signatures, serde annotations, and error message formats. The separation of validate_value (pure function in core-rust) from SchemaService (stateful wrapper in server-rust) is clean. Backward compatibility is carefully handled via serde defaults. The 3-wave sequential execution plan is appropriate for the dependency chain.

---

## Execution Summary

**Executed:** 2026-03-19
**Commits:** 2

### Files Created
- `packages/server-rust/src/service/domain/schema.rs` — SchemaService implementing SchemaProvider and ManagedService

### Files Modified
- `packages/core-rust/src/schema.rs` — Expanded FieldType enum (9 variants), FieldConstraint struct, FieldDef and MapSchema with new fields; added validate_value and validate_schema public functions; comprehensive unit tests
- `packages/core-rust/src/lib.rs` — Added FieldType, FieldConstraint, validate_value, validate_schema to re-exports
- `packages/core-rust/Cargo.toml` — Added `regex = "1"` dependency
- `packages/server-rust/src/service/domain/mod.rs` — Added `pub mod schema` and `pub use schema::SchemaService`

### Acceptance Criteria Status
- [x] 1. FieldType enum has all 9 variants with Debug, Clone, Default, PartialEq, Serialize, Deserialize; Any is the #[default] variant
- [x] 2. FieldConstraint has all 6 optional fields, derives Default, has #[serde(rename_all = "camelCase")]
- [x] 3. FieldDef has field_type: FieldType with #[serde(default)] and constraints: Option<FieldConstraint> with skip/default
- [x] 4. MapSchema has strict: bool with #[serde(default)]
- [x] 5. validate_value correctly validates: Valid for conforming data, Invalid for missing required, wrong type, constraint violation, unknown field (strict mode)
- [x] 6. validate_value returns Invalid when value is not a Value::Map
- [x] 7. FieldType::Binary matches Value::Bytes (not Value::Binary which does not exist)
- [x] 8. validate_schema returns Ok(()) for valid patterns; Err(errors) listing invalid patterns
- [x] 9. SchemaService::register_schema calls validate_schema before inserting; rejects schemas with invalid regex patterns
- [x] 10. SchemaService implements SchemaProvider: register (with pattern validation), retrieve, validate
- [x] 11. SchemaService::validate returns Valid when no schema is registered for the map (optional mode)
- [x] 12. core-rust/src/lib.rs re-exports FieldType, FieldConstraint, validate_value, and validate_schema
- [x] 13. All existing tests continue to pass (440 core + 551 server unit tests, 0 failures)

### Deviations
- [Rule 1 - Bug] Fixed 17 clippy warnings in schema.rs: uninlined format args, manual_let_else, match_same_arms (Int | Timestamp merged), cast_possible_truncation suppressed with allow + comment, missing_errors_doc added to validate_schema, must_use added to validate_value

### Notes
- Tests are co-located in #[cfg(test)] modules within each file (schema.rs for core-rust, schema.rs for server-rust)
- RequestContext::default() does not exist; tests construct it explicitly with literal fields
- 551 server tests include 11 new SchemaService tests; 440 core tests include 26 new schema validation tests

---

## Review History

### Review v1 (2026-03-19 12:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: FieldType has all 9 variants (String, Int, Float, Bool, Binary, Timestamp, Array, Map, Any) with Debug, Clone, Default, PartialEq, Serialize, Deserialize; Any annotated with #[default]
- [✓] AC2: FieldConstraint has all 6 optional fields (min_length, max_length, min_value, max_value, pattern, enum_values), derives Default, has #[serde(rename_all = "camelCase")], all fields have skip_serializing_if + default
- [✓] AC3: FieldDef has field_type: FieldType with #[serde(default)] and constraints: Option<FieldConstraint> with skip/default
- [✓] AC4: MapSchema has strict: bool with #[serde(default)]
- [✓] AC5: validate_value returns Valid for conforming data; returns Invalid with specific messages for missing required fields, wrong types, constraint violations, and unknown fields in strict mode — all covered by 26 unit tests
- [✓] AC6: validate_value returns Invalid{"expected a Map value"} for non-Map input
- [✓] AC7: FieldType::Binary matches Value::Bytes (the correct variant); explicitly tested in binary_type_accepts_bytes test
- [✓] AC8: validate_schema returns Ok(()) for valid patterns and Err(errors) with field name + pattern for invalid ones
- [✓] AC9: SchemaService::register_schema calls validate_schema before insert; register_schema_rejects_invalid_pattern test confirms rejection
- [✓] AC10: SchemaService fully implements SchemaProvider (get_schema, register_schema, validate, get_shape)
- [✓] AC11: SchemaService::validate returns Valid when no schema is registered (optional mode); validate_no_schema_returns_valid test confirms this
- [✓] AC12: lib.rs re-exports FieldType, FieldConstraint, validate_value, validate_schema alongside existing types
- [✓] AC13: 440 core + 551 server tests, 0 failures (verified by running tests)
- [✓] Build check: cargo test --release -p topgun-core passes (440 + 10 + 7 = 457 tests including doc-tests)
- [✓] Build check: cargo test --release -p topgun-server passes (551 unit + 4 integration = 555 tests)
- [✓] Clippy: cargo clippy -p topgun-core -p topgun-server -- -D warnings exits clean (0 warnings)
- [✓] SchemaService correctly uses DashMap for concurrent access; follows established pattern
- [✓] ManagedService impl has correct method signatures: name(), init(), reset(), shutdown(terminate: bool)
- [✓] serde wire format: MapSchema and FieldDef use snake_case (no rename_all); FieldConstraint uses camelCase as specified; no breaking changes
- [✓] Backward compatibility: old-format schema (without field_type/constraints/strict) deserializes correctly via #[serde(default)]
- [✓] MsgPack round-trip test (map_schema_msgpack_roundtrip) verifies serialization with new fields
- [✓] validate_value is a pure function (#[must_use] annotated); regex compiled per call as documented tradeoff
- [✓] get_shape returns None (stub as specified); RequestContext correctly imported and used in tests
- [✓] 5-file limit respected (4 modified + 1 created)
- [✓] No constraints violated (no CrdtService wiring, no Arrow types, no SyncShape logic, no CLI flags)
- [✓] Int-to-float widening coercion implemented for FieldType::Float accepting Value::Int

**Minor Issues:**
1. `field_defs` HashMap is built unconditionally in `validate_value` at line 211-212, but is only used inside the `if schema.strict` block (line 238). For the common case (strict=false), this allocates a HashMap that is never consumed. Consider moving the HashMap construction inside the `if schema.strict` block to avoid the allocation on the hot path.
2. `std::string::String` is used throughout `schema.rs` instead of the prelude-imported `String` (e.g., lines 63, 66, 80, 124, 137, 145, 149, 168-169, 208, 258, 276, 329). This is verbose but functionally equivalent — the prelude `String` and `std::string::String` are the same type. No behavioral impact.

**Summary:** The implementation is correct and complete. All 13 acceptance criteria are satisfied, all tests pass (991 total), clippy is clean, and the code follows established project patterns. The two minor issues are optimization and style points that do not affect correctness or functionality.

---

## Completion

**Completed:** 2026-03-19
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Delivered typed schema system for v2.0: FieldType enum (9 variants), FieldConstraint struct, validate_value/validate_schema pure functions in core-rust, and SchemaService implementing SchemaProvider trait in server-rust with optional-mode validation (no schema = passthrough).

### Key Files

- `packages/core-rust/src/schema.rs` — FieldType, FieldConstraint, MapSchema, FieldDef, validate_value, validate_schema + 26 unit tests
- `packages/server-rust/src/service/domain/schema.rs` — SchemaService (SchemaProvider + ManagedService) + 11 unit tests

### Patterns Established

- Optional-mode validation: no schema registered = passthrough (no validation). Enables gradual schema adoption.
- Pure validation function (`validate_value`) in core-rust, stateful service wrapper (`SchemaService`) in server-rust. Keeps core serializable and testable.
- Regex compiled per call in `validate_value` (pure, no cache), with `validate_schema` for registration-time pattern validation via `SchemaService::register_schema`.

### Deviations

- Fixed 17 clippy warnings (uninlined format args, manual_let_else, match_same_arms). Post-review: moved field_defs HashMap inside strict-only block, replaced std::string::String with String.
