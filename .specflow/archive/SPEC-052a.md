# SPEC-052a: Message Schema Foundation -- Base Types, Module Scaffold, Serde Renames

---
id: SPEC-052a
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-15
parent: SPEC-052
depends_on: [SPEC-049]
todo_ref: TODO-062
---

## Context

This is the foundation sub-spec of the Rust message schema compatibility work (SPEC-052). It establishes the `messages/` module scaffold, defines shared base types used by all other domain modules, and migrates existing Rust types (`Timestamp`, `LWWRecord`, `ORMapRecord`) to be wire-compatible with camelCase MsgPack field names.

The TS Zod schemas in `packages/core/src/schemas/` are the source of truth for the wire format. Rust `serde` structs must produce byte-identical MsgPack output when round-tripping through `rmp_serde::to_vec_named()` / `rmp_serde::from_slice()`.

### Critical Compatibility Issues (Inherited)

1. **Named vs positional encoding:** Must use `rmp_serde::to_vec_named()` (not `to_vec()`) for wire messages.
2. **Field naming:** TS uses camelCase; Rust needs `#[serde(rename_all = "camelCase")]`.
3. **Optional fields:** Use `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]` and `#[serde(default)]`.
4. **Dynamic values:** Use `rmpv::Value` for `z.any()` / `z.unknown()` fields at the wire boundary.

### Existing Rust Types Requiring Migration

- `Timestamp` (hlc.rs, line 24) -- needs `#[serde(rename_all = "camelCase")]`
- `LWWRecord<V>` (hlc.rs, line 305) -- needs `#[serde(rename_all = "camelCase")]`
- `ORMapRecord<V>` (hlc.rs, line 324) -- needs `#[serde(rename_all = "camelCase")]`

Existing tests use `rmp_serde::to_vec()` (array format) exclusively -- 15+ call sites. Adding `rename_all` affects `to_vec_named()` output but NOT `to_vec()`, so all existing tests continue to pass.

### Key Risk

The `hlc.rs` serde rename is the highest-risk change. Adding `rename_all = "camelCase"` changes `to_vec_named()` output. There appear to be zero existing call sites using `to_vec_named()` for Timestamp, so the risk is low.

## Goal

Establish the foundation for Rust message schema compatibility: module scaffold, shared base types, and wire-compatible serde on existing types.

### Observable Truths

1. `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` with `rename_all = "camelCase"` produce camelCase field names when serialized via `to_vec_named()`.
2. Existing Rust-only tests using `rmp_serde::to_vec()` (array format) continue to pass.
3. Base enums (`WriteConcern`, `ChangeEventType`, `PredicateOp`, `SortDirection`) serialize to their expected string values.
4. The `messages` module compiles and is accessible from the crate root.

## Task

Create the `messages/` module with base types and migrate existing types for wire compatibility.

### Approach

1. Add `rmpv = "1"` to `Cargo.toml` dependencies.
2. Add `#[serde(rename_all = "camelCase")]` to `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` in `hlc.rs`.
3. Create `messages/mod.rs` as module root with submodule declarations (initially just `base`; the `Message` enum will be completed in SPEC-052e after all domain modules exist).
4. Create `messages/base.rs` with shared types from `base-schemas.ts`.
5. Add `pub mod messages;` and re-exports to `lib.rs`.

## Requirements

### Domain 1: Base Types (~9 types)
**Source:** `base-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `WriteConcern` | `WriteConcernSchema` | Enum: FIRE_AND_FORGET, MEMORY, APPLIED, REPLICATED, PERSISTED |
| `ChangeEventType` | `ChangeEventTypeSchema` | Enum: ENTER, UPDATE, LEAVE. Shared across search, cluster, and client events domains. `SearchUpdateTypeSchema` is just an alias. |
| `PredicateOp` | `PredicateOpSchema` | Enum: eq, neq, gt, gte, lt, lte, like, regex, and, or, not |
| `PredicateNode` | `PredicateNodeSchema` | Recursive struct using `Option<Vec<PredicateNode>>` for children |
| `SortDirection` | `QuerySchema.sort` value enum | Enum: Asc, Desc. With `#[serde(rename_all = "lowercase")]` to match TS `z.enum(['asc', 'desc'])`. |
| `Query` | `QuerySchema` | Struct with predicate, sort, limit, cursor, where fields. Note: `where` is a Rust keyword, requires `#[serde(rename = "where")] pub r#where: Option<HashMap<String, rmpv::Value>>` |
| `ClientOp` | `ClientOpSchema` | Struct with id (optional), mapName, key, opType (optional), record (LWWRecord nullable/optional), orRecord (ORMapRecord nullable/optional), orTag (nullable/optional), writeConcern (optional), timeout (optional) |
| `AuthMessage` | `AuthMessageSchema` | type = "AUTH", token field, optional `protocolVersion` |
| `AuthRequiredMessage` | `AuthRequiredMessageSchema` | type = "AUTH_REQUIRED" |

### Serde Patterns

**Struct pattern:**
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInitMessage {
    pub map_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_sync_timestamp: Option<f64>,
}
```

**Dynamic value pattern:**
```rust
pub value: rmpv::Value,       // For z.any() / z.unknown()
pub value: Option<rmpv::Value>, // For z.any().nullable()
```

**Rust keyword field pattern:**
```rust
// For TS field "where" (Rust reserved keyword)
#[serde(rename = "where")]
pub r#where: Option<HashMap<String, rmpv::Value>>

// For TS field "type" (Rust reserved keyword)
#[serde(rename = "type")]
pub r#type: String
```

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/mod.rs` | Module declarations (base, plus placeholder comments for future submodules), re-exports |
| `packages/core-rust/src/messages/base.rs` | `WriteConcern`, `ChangeEventType`, `PredicateOp`, `SortDirection`, `PredicateNode`, `Query`, `ClientOp`, `AuthMessage`, `AuthRequiredMessage` |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/hlc.rs` | Add `#[serde(rename_all = "camelCase")]` to `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` |
| `packages/core-rust/src/lib.rs` | Add `pub mod messages;` and re-exports |
| `packages/core-rust/Cargo.toml` | Add `rmpv = "1"` dependency |

**Total: 2 new + 3 modified = 5 files** (at Language Profile limit)

## Acceptance Criteria

1. **AC-5 (from parent): Existing Rust types compatible.** `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` with `#[serde(rename_all = "camelCase")]` produce MsgPack (via `to_vec_named()`) with camelCase field names that TS can decode. Existing Rust-only tests using `rmp_serde::to_vec()` (array format) continue to pass.

2. **AC-7 (from parent): cargo test passes.** All existing core-rust tests pass. New base type serde round-trip unit tests pass. No regressions.

3. **AC-base-compile: Module compiles.** `pub mod messages;` in `lib.rs` compiles successfully. Base types are accessible from the crate root via re-exports.

4. **AC-base-roundtrip: Base types round-trip.** Unit tests verify serde round-trip for `WriteConcern`, `ChangeEventType`, `PredicateOp`, `SortDirection`, `PredicateNode`, `Query`, `ClientOp`, `AuthMessage`, `AuthRequiredMessage` using `to_vec_named()` / `from_slice()`.

5. **AC-lww-rmpv-roundtrip: LWWRecord<rmpv::Value> round-trip.** Unit test validates Assumption A4 by round-tripping a `LWWRecord<rmpv::Value>` instance through `to_vec_named()` / `from_slice()` with nested dynamic values.

## Constraints

- Do NOT implement message handler logic -- strictly struct definitions and serde configuration.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT replace the existing `Value` enum in `types.rs` -- use `rmpv::Value` for wire-boundary dynamic values.
- Do NOT add `rmpv` dependency to anything outside `core-rust`.
- Max 5 files modified/created.

## Assumptions

- `rmpv` 1.x is compatible with the `rmp-serde` 1.x already in use.
- Adding `rename_all = "camelCase"` to `Timestamp` etc. does NOT affect `to_vec()` (array format) serialization -- only `to_vec_named()` output changes.
- `z.number()` maps to `f64` in Rust for most cases. `Timestamp.millis` and `counter` are always MsgPack integers on the wire (u64/u32).
- `LWWRecord<V>` has dual usage: wire boundary uses `LWWRecord<rmpv::Value>`, CRDT layer uses `LWWRecord<crate::Value>`.

## Audit History

### Audit v1 (2026-02-15)
**Status:** NEEDS_REVISION

**Context Estimate:** ~20% total (small spec, 5 files, straightforward types)

**Critical:**
1. **ClientOp field list is wrong.** Spec says "Struct with mapName, key, value, timestamp, writeConcern, ttlMs fields" but the actual TS `ClientOpSchema` in `base-schemas.ts` has completely different fields: `id` (optional), `mapName`, `key`, `opType` (optional), `record` (LWWRecord, nullable/optional), `orRecord` (ORMapRecord, nullable/optional), `orTag` (nullable/optional), `writeConcern` (optional), `timeout` (optional). The spec's field list would produce a Rust struct incompatible with the TS wire format, directly violating the "Rust must conform to what TS already produces" constraint.

2. **Query field list includes non-existent `offset` field.** Spec says "Struct with predicate, sort, limit, offset, cursor fields" but the actual TS `QuerySchema` does not have an `offset` field. The TS source code contains a comment: `cursor: z.string().optional(), // Replaces offset for pagination`. Including `offset` would produce extra bytes on the wire that TS would not expect.

3. **Query field list omits `where` field.** The TS `QuerySchema` has `where: z.record(z.string(), z.any()).optional()` which is not mentioned in the spec. Since `where` is a Rust reserved keyword, this field requires special serde handling: `#[serde(rename = "where")] pub r#where: Option<HashMap<String, rmpv::Value>>` or similar. Omitting it would produce an incomplete Rust struct that cannot round-trip Query messages from TS.

**Recommendations:**
4. **PredicateNode children type.** Spec says `Box<Vec<PredicateNode>>` for children. `Vec<T>` already allocates on the heap, so `Box<Vec<T>>` adds unnecessary indirection. Since `PredicateNode` contains `Option<Vec<PredicateNode>>` and Vec has a known fixed size (pointer + length + capacity), the type is not infinitely sized. Recommend `Option<Vec<PredicateNode>>` without Box.

5. **Query.sort type needs clarification.** TS defines `sort` as `z.record(z.string(), z.enum(['asc', 'desc']))`. The Rust mapping should be explicit: either `HashMap<String, String>` (simple) or `HashMap<String, SortDirection>` with a `SortDirection` enum (type-safe). A `SortDirection` enum (`Asc`, `Desc`) with `#[serde(rename_all = "lowercase")]` would be the idiomatic Rust approach and prevent invalid values.

6. [Strategic] **Assumption A4 fragility.** The assumption "LWWRecord<V> has dual usage: wire boundary uses LWWRecord<rmpv::Value>, CRDT layer uses LWWRecord<crate::Value>" is stated but not tested in this spec's acceptance criteria. Consider adding a round-trip test for `LWWRecord<rmpv::Value>` specifically to validate this assumption early.

### Response v1 (2026-02-15)
**Applied:** All 6 items (3 critical + 3 recommendations)

**Changes:**
1. [✓] ClientOp field list corrected — updated to match actual TS ClientOpSchema: id (optional), mapName, key, opType (optional), record (nullable/optional LWWRecord), orRecord (nullable/optional ORMapRecord), orTag (nullable/optional string), writeConcern (optional), timeout (optional)
2. [✓] Query offset field removed — TS QuerySchema uses cursor for pagination, not offset
3. [✓] Query where field added — included with Rust keyword handling pattern: `#[serde(rename = "where")] pub r#where: Option<HashMap<String, rmpv::Value>>`
4. [✓] PredicateNode children type — changed from `Box<Vec<PredicateNode>>` to `Option<Vec<PredicateNode>>` (no Box needed)
5. [✓] SortDirection enum added — type-safe `Asc`/`Desc` enum with `#[serde(rename_all = "lowercase")]`, used in Query.sort as `HashMap<String, SortDirection>`
6. [✓] LWWRecord<rmpv::Value> round-trip test — added AC-lww-rmpv-roundtrip acceptance criterion to validate Assumption A4 early

### Audit v2 (2026-02-15)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Verification against TS source (`packages/core/src/schemas/base-schemas.ts`):**
- WriteConcern: 5 variants match TS `WriteConcernSchema` exactly.
- ChangeEventType: 3 variants (ENTER, UPDATE, LEAVE) match TS `ChangeEventTypeSchema`. Confirmed `SearchUpdateTypeSchema` is an alias (search-schemas.ts:48).
- PredicateOp: 11 variants match TS `PredicateOpSchema` exactly.
- PredicateNode: 4 fields (op, attribute, value, children) match TS `PredicateNodeSchema`. Recursive `Option<Vec<PredicateNode>>` is correct.
- Query: 5 fields (where, predicate, sort, limit, cursor) match TS `QuerySchema` exactly. No `offset` field. `where` keyword handling documented.
- ClientOp: 9 fields match TS `ClientOpSchema` exactly (id, mapName, key, opType, record, orRecord, orTag, writeConcern, timeout).
- AuthMessage: 3 fields (type, token, protocolVersion) match TS `AuthMessageSchema`.
- AuthRequiredMessage: 1 field (type) matches TS `AuthRequiredMessageSchema`.

**Verification against Rust source (`packages/core-rust/src/hlc.rs`):**
- Timestamp (line 24): `millis: u64`, `counter: u32`, `node_id: String`. Currently has no `rename_all`. Adding `camelCase` will rename `node_id` to `nodeId` in `to_vec_named()` output, matching TS `nodeId`. Confirmed zero `to_vec_named()` call sites in core-rust.
- LWWRecord (line 305): `value: Option<V>`, `timestamp: Timestamp`, `ttl_ms: Option<u64>`. Adding `camelCase` will rename `ttl_ms` to `ttlMs`, matching TS `ttlMs`.
- ORMapRecord (line 324): `value: V`, `timestamp: Timestamp`, `tag: String`, `ttl_ms: Option<u64>`. Same `ttlMs` rename applies.

**Verification against Cargo.toml:**
- `rmp-serde = "1"` already present. `rmpv = "1"` to be added. Compatible crate family (both from `rmp` ecosystem).

**Language profile:** Compliant with Rust profile (5 files at limit, type definitions are pure data -- no trait-first ordering needed for structs-only spec).

**Project compliance:** Honors PROJECT.md decisions (MsgPack wire format, Rust migration, TS as source of truth).

**Strategic fit:** Aligned with project goals (Rust migration Phase 2, wire compatibility is prerequisite for all subsequent SPEC-052 sub-specs).

**Recommendations:**
1. **AuthMessage/AuthRequiredMessage `type` keyword handling.** The `type` field in these structs is a Rust reserved keyword (like `where` in Query). The spec documents the `where` keyword pattern in Serde Patterns but does not explicitly note that `type` also needs `#[serde(rename = "type")] pub r#type: String`. An implementer would likely figure this out, but adding it to the Serde Patterns section would make it explicit and consistent.

2. **Domain 1 heading says "~11 types" but table lists 9.** The actual count is 9 new types (plus 3 existing types being modified = 12 total touched). Minor cosmetic inconsistency.

**Comment:** Spec is well-structured with clear TS-to-Rust mappings, thorough serde pattern documentation, and precise acceptance criteria. All v1 critical issues were properly addressed. Cross-referencing against actual TS and Rust source confirms correctness of all type definitions.

### Response v2 (2026-02-15)
**Applied:** All 2 recommendations

**Changes:**
1. [✓] AuthMessage/AuthRequiredMessage `type` keyword handling — added `#[serde(rename = "type")] pub r#type: String` pattern to Serde Patterns section alongside existing `where` keyword pattern
2. [✓] Domain 1 heading count — corrected from "~11 types" to "~9 types" to match the actual table count (9 new types)

### Audit v3 (2026-02-15)
**Status:** APPROVED

**Context Estimate:** ~17% total

**Independent verification against source files:**

Verified all 9 type definitions against TS source (`packages/core/src/schemas/base-schemas.ts`):
- WriteConcern: 5 variants match (FIRE_AND_FORGET, MEMORY, APPLIED, REPLICATED, PERSISTED)
- ChangeEventType: 3 variants match (ENTER, UPDATE, LEAVE)
- PredicateOp: 11 variants match (eq, neq, gt, gte, lt, lte, like, regex, and, or, not)
- PredicateNode: 4 fields match (op, attribute, value, children) with recursive Option<Vec>
- SortDirection: 2 variants match TS `z.enum(['asc', 'desc'])` with lowercase rename
- Query: 5 fields match (where, predicate, sort, limit, cursor) -- no offset, keyword handling documented
- ClientOp: 9 fields match (id, mapName, key, opType, record, orRecord, orTag, writeConcern, timeout)
- AuthMessage: 3 fields match (type, token, protocolVersion) -- keyword handling documented
- AuthRequiredMessage: 1 field matches (type) -- keyword handling documented

Verified Rust source (`packages/core-rust/src/hlc.rs`):
- Timestamp (line 24): fields `millis: u64`, `counter: u32`, `node_id: String` -- no existing `rename_all`
- LWWRecord (line 300-312): has `#[serde(bound(...))]` for generics -- adding `rename_all` is compatible
- ORMapRecord (line 319-333): same `serde(bound)` pattern -- compatible
- Confirmed zero `to_vec_named()` call sites across entire core-rust package (20 `to_vec()` calls, all array format)

Verified Cargo.toml: `rmp-serde = "1"` present, no `rmpv` yet. Addition is straightforward.

Verified no existing `messages/` directory or module.

**All 10 audit dimensions passed:**
- Clarity: Precise type-by-type mapping with field lists and serde patterns
- Completeness: All files listed, all types enumerated, keyword edge cases documented
- Testability: 5 measurable acceptance criteria with specific methods and types
- Scope: Clear boundaries (no handler logic, no TS changes, max 5 files)
- Feasibility: Sound approach, all assumptions verified or explicitly tested
- Architecture fit: Follows existing crate module structure
- Non-duplication: rmpv::Value for wire boundary, existing Value for CRDT layer -- distinct purposes
- Cognitive load: Straightforward type definitions with serde attributes
- Strategic fit: Foundation for SPEC-052b/c/d/e, aligned with Rust migration Phase 2
- Project compliance: Honors MsgPack wire format, TS as source of truth, Rust Language Profile (5 files at limit)

**Comment:** Spec is thorough and implementation-ready. All previous audit issues have been resolved. Both Rust keyword patterns (`where`, `type`) are documented in the Serde Patterns section. Type count heading is accurate. No issues found.

---

## Execution Summary

**Executed:** 2026-02-15
**Commits:** 2

### Files Created
- `packages/core-rust/src/messages/mod.rs` -- Module root with submodule declarations (base), re-exports of all 9 base types, placeholder comments for future submodules
- `packages/core-rust/src/messages/base.rs` -- 9 wire-compatible types: WriteConcern, ChangeEventType, PredicateOp, SortDirection, PredicateNode, Query, ClientOp, AuthMessage, AuthRequiredMessage. Includes 26 unit tests for serde round-trips.

### Files Modified
- `packages/core-rust/Cargo.toml` -- Added `rmpv = { version = "1", features = ["with-serde"] }` dependency
- `packages/core-rust/src/hlc.rs` -- Added `#[serde(rename_all = "camelCase")]` to Timestamp, LWWRecord<V>, ORMapRecord<V>
- `packages/core-rust/src/lib.rs` -- Added `pub mod messages;` and re-exports for all 9 base types

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC-5: Existing Rust types compatible. Timestamp, LWWRecord, ORMapRecord produce camelCase field names via to_vec_named(). All 173 existing tests pass (to_vec array format unaffected).
- [x] AC-7: cargo test passes. 199 tests pass (173 existing + 26 new) plus 6 doc-tests. No regressions.
- [x] AC-base-compile: Module compiles. `pub mod messages;` in lib.rs compiles. All 9 types accessible from crate root via re-exports.
- [x] AC-base-roundtrip: Base types round-trip. Unit tests verify serde round-trip for all 9 types using to_vec_named()/from_slice().
- [x] AC-lww-rmpv-roundtrip: LWWRecord<rmpv::Value> round-trip. Two tests validate Assumption A4 (nested dynamic values and tombstone case).

### Deviations
1. [Rule 3 - Blocking] rmpv crate requires `features = ["with-serde"]` to enable Serialize/Deserialize impls on rmpv::Value. Spec said `rmpv = "1"` but that doesn't include serde support. Fixed to `rmpv = { version = "1", features = ["with-serde"] }`.
2. [Rule 1 - Bug] `Option<Option<T>>` with `skip_serializing_if = "Option::is_none"` and `default` collapses `Some(None)` to `None` on round-trip. Added `deserialize_double_option` helper function to correctly distinguish absent fields (None) from explicitly-null fields (Some(None)) for ClientOp's nullable+optional fields (record, orRecord, orTag).
3. [Rule 1 - Bug] WriteConcern enum variants use SCREAMING_CASE to match TS wire format, which triggers `non_camel_case_types` warning. Added `#[allow(non_camel_case_types)]` since the variant names must match the wire format exactly.

### Notes
- The `deserialize_double_option` helper will be reusable by SPEC-052b/c/d for other nullable+optional fields.
- Timestamp import is only needed in test code, so it is imported in the `#[cfg(test)]` module rather than the main module to avoid unused-import warnings.

---

## Review History

### Review v1 (2026-02-15)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Major:**
1. **Clippy pedantic lint failures (6 errors)**
   - File: `packages/core-rust/src/messages/mod.rs:1`
   - File: `packages/core-rust/src/messages/mod.rs:4`
   - File: `packages/core-rust/src/messages/base.rs:6`
   - File: `packages/core-rust/src/messages/base.rs:27`
   - File: `packages/core-rust/src/messages/base.rs:42`
   - File: `packages/core-rust/src/messages/base.rs:189`
   - Issue: `cargo clippy -- -D warnings` fails with 6 errors. The workspace enables `clippy::pedantic = "warn"` and the CI command uses `-D warnings`, making all warnings errors. Specifically:
     (a) `doc_markdown` lint: `TopGun`, `MsgPack` (2 occurrences), `SCREAMING_CASE`, and `AUTH_REQUIRED` need backticks in doc comments (5 instances across mod.rs and base.rs).
     (b) `option_option` lint: The `deserialize_double_option` function returns `Result<Option<Option<T>>, D::Error>` which triggers the pedantic `option_option` lint. This is an intentional design choice (documented in PROJECT.md as the "double-option deserialization pattern") and needs `#[allow(clippy::option_option)]`.
   - Fix: (a) Wrap `TopGun`, `MsgPack`, `SCREAMING_CASE`, `AUTH_REQUIRED` in backticks in doc comments. (b) Add `#[allow(clippy::option_option)]` on the `deserialize_double_option` function and the three `Option<Option<T>>` fields in `ClientOp`.

**Passed:**
- [x] AC-5: Existing Rust types compatible -- `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` all have `#[serde(rename_all = "camelCase")]` and produce correct camelCase field names via `to_vec_named()`. All 173 existing tests pass (array format unaffected).
- [x] AC-7: cargo test passes -- 199 tests pass (173 existing + 26 new) plus 6 doc-tests. No regressions.
- [x] AC-base-compile: Module compiles -- `pub mod messages;` in `lib.rs` compiles. All 9 types accessible from crate root via re-exports in `lib.rs` lines 51-54.
- [x] AC-base-roundtrip: Base types round-trip -- 26 unit tests verify serde round-trip for all 9 types using `to_vec_named()`/`from_slice()`, including enum string verification, camelCase field name verification, recursive predicate nodes, and nullable+optional field edge cases.
- [x] AC-lww-rmpv-roundtrip: LWWRecord<rmpv::Value> round-trip -- Two tests validate Assumption A4: `lww_record_rmpv_value_roundtrip` (nested dynamic values) and `lww_record_rmpv_value_tombstone_roundtrip` (tombstone case).
- [x] Wire compatibility: All 9 types independently verified against TS source (`packages/core/src/schemas/base-schemas.ts`). Field names, types, optionality, and nullability all match.
- [x] Constraints respected: No handler logic, no TS format changes, `rmpv` only in `core-rust`, 5 files exactly.
- [x] Deviations well-documented: All 3 deviations (rmpv serde feature, double-option helper, non_camel_case allow) are justified and properly recorded.
- [x] Code quality: Clean type definitions, thorough WHY-comments (no spec references in code), proper separation of test helper from production code, `deserialize_double_option` is reusable for SPEC-052b/c/d.
- [x] No unnecessary `.clone()`, `.unwrap()`, or `.expect()` in production code.
- [x] No security issues (pure data definitions, no I/O, no user input handling).

**Summary:** Implementation is functionally complete and correct. All 5 acceptance criteria are met. All 9 types match the TS source of truth exactly. The only issue is that `cargo clippy -- -D warnings` fails with 6 pedantic lint errors (5 doc_markdown + 1 option_option). These are mechanical fixes that do not affect correctness, but they must be resolved since the workspace enforces pedantic clippy as errors.

### Fix Response v1 (2026-02-15)
**Applied:** All issues from Review v1

**Fixes:**
1. [✓] Clippy pedantic lint failures (6 errors) — All 6 resolved:
   - (a) Wrapped `TopGun`, `MsgPack` (x2), `SCREAMING_CASE`, `AUTH_REQUIRED` in backticks in doc comments (5 `doc_markdown` lints)
   - (b) Added `#[allow(clippy::option_option)]` on `deserialize_double_option` function and `ClientOp` struct (1 `option_option` lint)
   - Commit: 063a7f1

### Review v2 (2026-02-15)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC-5: Existing Rust types compatible -- `Timestamp` (hlc.rs:24), `LWWRecord<V>` (hlc.rs:302), `ORMapRecord<V>` (hlc.rs:322) all have `#[serde(rename_all = "camelCase")]`. Tests at base.rs:539-604 verify camelCase field names (`nodeId`, `ttlMs`) via `to_vec_named()`. All 173 existing tests using `to_vec()` array format pass unchanged.
- [x] AC-7: cargo test passes -- 199 tests + 6 doc-tests all pass. Zero failures, zero regressions.
- [x] AC-base-compile: Module compiles -- `cargo check` succeeds. `pub mod messages;` in lib.rs:20. All 9 types re-exported at lib.rs:51-54 and accessible from crate root.
- [x] AC-base-roundtrip: Base types round-trip -- 26 unit tests in base.rs:200-655 cover all 9 types via `to_vec_named()`/`from_slice()`. Includes enum string verification, camelCase field name verification, recursive predicate nodes, full/minimal struct variants, and nullable+optional edge cases.
- [x] AC-lww-rmpv-roundtrip: LWWRecord<rmpv::Value> round-trip -- Two tests at base.rs:609-654 validate Assumption A4: nested dynamic values (Map with strings, integers, arrays) and tombstone case (value: None, ttl_ms: None).
- [x] Wire compatibility verified -- All 9 Rust types independently compared field-by-field against TS source (`packages/core/src/schemas/base-schemas.ts`). Every field name, type, optionality pattern (.optional() vs .nullable().optional()), and enum variant matches exactly.
- [x] Constraints respected -- No handler logic implemented. No TS format changes. `rmpv` only in core-rust Cargo.toml. Exactly 5 files touched (2 created + 3 modified).
- [x] Language profile checks passed -- `cargo check`: success. `cargo clippy -- -D warnings`: zero warnings/errors. `cargo test`: 199 passed + 6 doc-tests passed.
- [x] Rust idioms followed -- No unnecessary `.clone()` in production code. No `.unwrap()` or `.expect()` in production code (only in tests). No `unsafe` blocks. `#[allow(non_camel_case_types)]` and `#[allow(clippy::option_option)]` justified by wire format requirements.
- [x] Code quality -- Clean separation of types, well-documented with WHY-comments, no spec/phase references in code. `deserialize_double_option` helper is well-explained and reusable. Test coverage is thorough with edge cases.
- [x] Architecture alignment -- Follows existing crate module structure. Uses `rmpv::Value` for wire boundary, existing `Value` for CRDT layer (distinct purposes, no duplication).
- [x] Deviations well-documented -- All 3 deviations properly classified and justified in Execution Summary.
- [x] No security issues -- Pure data definitions with serde attributes. No I/O, no user input handling, no secrets.
- [x] Review v1 fixes verified -- All 6 clippy pedantic lint errors resolved. Doc comments use backticks for `TopGun`, `MsgPack`, `SCREAMING_CASE`, `AUTH_REQUIRED`. `#[allow(clippy::option_option)]` applied on both the helper function and the `ClientOp` struct.

**Summary:** Implementation is complete, correct, and clean. All 5 acceptance criteria are met. All Review v1 issues have been resolved. The code passes all three language profile checks (build, lint, test) with zero warnings. All 9 types match the TS wire format exactly. No critical, major, or minor issues found.

---

## Completion

**Completed:** 2026-02-15
**Total Commits:** 3 (2 implementation + 1 clippy fix)
**Audit Cycles:** 3
**Review Cycles:** 2

---
*Child of SPEC-052. Created by /sf:split on 2026-02-15.*
