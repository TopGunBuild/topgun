# SPEC-054: Message Schema Architecture Fix-on-Port

---
id: SPEC-054
type: refactor
status: done
priority: P0
complexity: medium
created: 2026-02-16
depends_on: [SPEC-052a, SPEC-052b]
todo_ref: TODO-079
---

## Context

Post-execution review of SPEC-052a and SPEC-052b revealed three architectural issues that must be resolved before continuing with SPEC-052c/d/e. The current Rust message structs copy JavaScript limitations instead of leveraging Rust's type system:

1. **`r#type: String` conflicts with SPEC-052e's Message enum.** SPEC-052e plans `#[serde(tag = "type")]` on a `Message` enum. If inner structs also have `r#type: String`, serde produces duplicate `type` keys -- undefined behavior in MsgPack. The enum owns the discriminator tag; inner structs must not contain it.

2. **`f64` for integer fields copies JS's lack of integer types.** JS has only `number` (IEEE 754 float64). Rust has proper integers. The TS `msgpackr` library encodes integer-valued `number`s as MsgPack integers (not floats). When Rust uses `f64`, `rmp_serde` emits MsgPack float64 on re-serialization -- a different binary format. Using `u32`/`u64` produces identical MsgPack encoding to what TS emits.

3. **No `Default` derives** on payload structs with many optional fields, forcing verbose construction in tests and handlers.

### Scope of Impact

| File | `r#type: String` fields to remove | `f64` fields to fix | `Default` candidates |
|------|-----------------------------------|---------------------|---------------------|
| `base.rs` | `AuthMessage.r#type`, `AuthRequiredMessage.r#type` | `ClientOp.timeout`, `AuthMessage.protocol_version`, `Query.limit` | `ClientOp`, `Query` |
| `sync.rs` | 18 structs: all `*Message` types + `ORMapSyncInit`, `BatchMessage` | `OpBatchPayload.timeout`, `SyncInitMessage.last_sync_timestamp`, `SyncRespRootPayload.root_hash`, `SyncRespBucketsPayload.buckets` (values), `ORMapSyncInit.root_hash`/`bucket_hashes`/`last_sync_timestamp`, `ORMapSyncRespRootPayload.root_hash`, `ORMapSyncRespBucketsPayload.buckets`, `OpRejectedPayload.code`, `BatchMessage.count` | `OpBatchPayload`, `OpAckPayload` |
| `query.rs` | 3 structs: `QuerySubMessage`, `QueryUnsubMessage`, `QueryRespMessage` | (none) | `QueryRespPayload` |

## Goal

Establish the correct Rust-idiomatic struct pattern for all message types so that: (a) inner structs do not own a `type` field (enabling SPEC-052e's tagged enum), (b) numeric fields use proper integer types with verified MsgPack wire compatibility, and (c) payload structs with optional fields derive `Default`.

### Goal-Backward Analysis

**Outcome:** All existing message structs are ready for SPEC-052e's `#[serde(tag = "type")] Message` enum without modification. Future SPEC-052c/d structs follow the corrected pattern from the start.

**Observable Truths:**

1. No Rust message struct in `base.rs`, `sync.rs`, or `query.rs` contains a `type` (or `r#type`) field.
2. A prototype `#[serde(tag = "type")]` enum wrapping 3 representative variants (payload-wrapped, flat, flat+binary) round-trips correctly through `rmp_serde`.
3. Hash fields (`root_hash`, `buckets` values, `bucket_hashes` values) use `u32` and round-trip correctly when TS sends integer-valued MsgPack.
4. Count/code fields (`count`, `code`) use `u32`.
5. Timestamp-millisecond fields (`last_sync_timestamp`, `timeout`) use `u64`.
6. Payload structs with optional fields implement `Default`.
7. All 232+ existing tests pass after modifications (zero regressions).

**Key Links (fragile connections):**

- Removing `r#type` from inner structs changes the struct layout. Every test that constructs a message struct with `r#type: "..."` must be updated.
- Changing `f64` to integer types changes the serialized MsgPack format. Round-trip tests using `.0` float literals must be updated to integer literals.
- The prototype `#[serde(tag = "type")]` enum must prove that `rmp_serde` handles internally-tagged enums with MsgPack maps (not arrays). This is the critical feasibility gate for SPEC-052e.

## Task

Refactor all SPEC-052a and SPEC-052b message structs to remove `r#type: String`, replace `f64` with proper integer types, add `Default` derives, and prototype the `#[serde(tag = "type")]` Message enum pattern.

### Approach

1. **Prototype first:** Create a `#[cfg(test)]` prototype of the `#[serde(tag = "type")]` Message enum with 3 representative variants to verify serde behavior with `rmp_serde`. This validates the architectural direction before touching production structs.

2. **Remove `r#type: String`** from all 23 message/flat structs across `base.rs`, `sync.rs`, and `query.rs`.

3. **Replace `f64` with integer types** according to the type policy (see Requirements).

4. **Add `#[derive(Default)]`** to payload structs with many optional fields.

5. **Update all tests** to remove `r#type` construction, update float literals to integers, verify round-trips.

## Requirements

### R1: Integer Type Policy

| Field pattern | Current type | New Rust type | Rationale |
|---------------|-------------|---------------|-----------|
| `root_hash` (LWW + ORMap) | `f64` | `u32` | FNV-1a returns 32-bit unsigned (`hash >>> 0`); combineHashes also returns `>>> 0` |
| `buckets` / `bucket_hashes` values | `HashMap<String, f64>` | `HashMap<String, u32>` | Same as root_hash -- bucket hashes are 32-bit unsigned |
| `count` (BatchMessage) | `f64` | `u32` | Message count; always non-negative integer |
| `code` (OpRejectedPayload) | `f64` | `u32` | HTTP-style error code; always non-negative integer |
| `timeout` (ClientOp, OpBatchPayload) | `f64` | `u64` | Milliseconds; can exceed u32 range for long timeouts |
| `last_sync_timestamp` (SyncInitMessage, ORMapSyncInit) | `f64` | `u64` | Milliseconds since epoch; matches `Timestamp.millis: u64` |
| `protocol_version` (AuthMessage) | `f64` | `u32` | Protocol version number; small non-negative integer |
| `limit` (Query) | `f64` | `u32` | Pagination limit; non-negative integer |
| `ttl_ms` (LWWRecord, ORMapRecord) | `Option<u64>` | No change | Already correct in hlc.rs |
| `Timestamp.millis` | `u64` | No change | Already correct in hlc.rs |
| `Timestamp.counter` | `u32` | No change | Already correct in hlc.rs |

**MsgPack compatibility:** TS `msgpackr` encodes integer-valued `number`s (e.g., `42`) as MsgPack positive integer format (1/2/4/8 bytes depending on magnitude), NOT as float64. Rust `rmp_serde` deserializes MsgPack integers into `u32`/`u64` directly. Conversely, Rust `u32`/`u64` serializes as MsgPack integer -- identical binary format to what TS produces. Using `f64` on the Rust side would produce MsgPack float64 format on re-serialization, which is a different binary encoding (even though the value is numerically equal).

### R2: Remove `r#type: String` from All Message Structs

Every struct that currently has `#[serde(rename = "type")] pub r#type: String` must have that field removed. The `type` discriminator will be owned by the `Message` enum (SPEC-052e) via `#[serde(tag = "type")]`.

**Structs to modify in `base.rs`:**

| Struct | Field to remove |
|--------|----------------|
| `AuthMessage` | `pub r#type: String` |
| `AuthRequiredMessage` | `pub r#type: String` |

**Structs to modify in `sync.rs`:**

| Struct | Field to remove |
|--------|----------------|
| `ClientOpMessage` | `pub r#type: String` |
| `OpBatchMessage` | `pub r#type: String` |
| `SyncInitMessage` | `pub r#type: String` |
| `SyncRespRootMessage` | `pub r#type: String` |
| `SyncRespBucketsMessage` | `pub r#type: String` |
| `SyncRespLeafMessage` | `pub r#type: String` |
| `MerkleReqBucketMessage` | `pub r#type: String` |
| `ORMapSyncInit` | `pub r#type: String` |
| `ORMapSyncRespRoot` | `pub r#type: String` |
| `ORMapSyncRespBuckets` | `pub r#type: String` |
| `ORMapMerkleReqBucket` | `pub r#type: String` |
| `ORMapSyncRespLeaf` | `pub r#type: String` |
| `ORMapDiffRequest` | `pub r#type: String` |
| `ORMapDiffResponse` | `pub r#type: String` |
| `ORMapPushDiff` | `pub r#type: String` |
| `OpAckMessage` | `pub r#type: String` |
| `OpRejectedMessage` | `pub r#type: String` |
| `BatchMessage` | `pub r#type: String` |

**Structs to modify in `query.rs`:**

| Struct | Field to remove |
|--------|----------------|
| `QuerySubMessage` | `pub r#type: String` |
| `QueryUnsubMessage` | `pub r#type: String` |
| `QueryRespMessage` | `pub r#type: String` |

**Note:** Payload structs (e.g., `OpBatchPayload`, `SyncRespRootPayload`) do NOT have a `r#type` field and require no changes for this requirement.

### R3: Prototype Tagged Enum

Add a `#[cfg(test)]` module (in `mod.rs` or a dedicated test file) containing a prototype `Message` enum with 3 representative variants:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum MessagePrototype {
    // Payload-wrapped variant
    #[serde(rename = "OP_ACK")]
    OpAck { payload: OpAckPayload },

    // Flat variant (fields inline)
    #[serde(rename = "SYNC_INIT")]
    SyncInit {
        #[serde(rename = "mapName")]
        map_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        #[serde(rename = "lastSyncTimestamp")]
        last_sync_timestamp: Option<u64>,
    },

    // Flat variant with binary data
    #[serde(rename = "BATCH")]
    Batch {
        count: u32,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
}
```

Tests must verify:
- Round-trip for each variant pattern
- Serialized MsgPack contains `type` key with correct string discriminator
- No duplicate `type` keys (inner structs have no `type` field)
- Deserialization from a MsgPack map with `type` key correctly dispatches to the right variant

### R4: Add `Default` Derives

Add `#[derive(Default)]` to these payload/flat structs:

| Struct | File | Justification |
|--------|------|---------------|
| `ClientOp` | `base.rs` | 7 optional fields out of 9 total |
| `Query` | `base.rs` | 5 optional fields out of 5 total (all optional) |
| `OpBatchPayload` | `sync.rs` | 2 optional fields; `ops` defaults to empty Vec |
| `OpAckPayload` | `sync.rs` | 2 optional fields out of 3 |
| `QueryRespPayload` | `query.rs` | 3 optional fields out of 5 |

**Constraint:** `Default` requires all field types to implement `Default`. `String` and `Vec<T>` do; `WriteConcern` and `bool` do. For structs with required non-defaultable fields (like `op_id: String`), `Default` will produce empty strings -- this is acceptable for test/builder convenience, not for production construction.

**Note:** `ClientOp` Default derive produces semantically invalid defaults (`map_name: ""`, `key: ""`). The executor should add a doc comment `/// Note: Default produces empty map_name/key -- for test convenience only` on the struct.

### Files to Modify

| File | Changes |
|------|---------|
| `packages/core-rust/src/messages/base.rs` | Remove `r#type` from 2 structs, change 3 `f64` fields to integer types, add `Default` to 2 structs (including doc comment for `ClientOp`), update tests |
| `packages/core-rust/src/messages/sync.rs` | Remove `r#type` from 18 structs, change ~12 `f64` fields to integer types, add `Default` to 2 structs, update tests |
| `packages/core-rust/src/messages/query.rs` | Remove `r#type` from 3 structs, add `Default` to 1 struct, update tests |
| `packages/core-rust/src/messages/mod.rs` | Add `#[cfg(test)]` prototype Message enum with tests |

**Total: 4 files modified, 0 files created**

## Acceptance Criteria

1. **AC-type-removed:** No struct in `base.rs`, `sync.rs`, or `query.rs` contains a field named `r#type` or `type` (verified by `grep -r 'r#type\|"type"' packages/core-rust/src/messages/{base,sync,query}.rs` returning zero matches for field definitions).

2. **AC-integer-types:** All fields listed in the Integer Type Policy table use their specified Rust integer types. Specifically: `root_hash: u32`, `buckets: HashMap<String, u32>`, `count: u32`, `code: Option<u32>`, `timeout: Option<u64>`, `last_sync_timestamp: Option<u64>`, `protocol_version: Option<u32>`, `limit: Option<u32>`.

3. **AC-prototype-roundtrip:** A `#[cfg(test)]` prototype `#[serde(tag = "type")]` enum with 3 variants (payload-wrapped, flat, flat+binary) round-trips through `rmp_serde::to_vec_named()` / `rmp_serde::from_slice()`. Serialized output contains exactly one `type` key with the correct discriminator string.

4. **AC-integer-wire-compat:** A test verifies that Rust `u32` serialized via `rmp_serde::to_vec_named()` produces MsgPack integer format (not float64). Specifically: serialize a struct with a `u32` field, inspect the raw bytes or deserialize via `rmpv::Value` and assert the value is `Value::Integer`, not `Value::F64`.

5. **AC-default-derives:** Structs listed in R4 derive `Default`. A test constructs each via `T::default()` without error.

6. **AC-all-tests-pass:** `cargo test` passes with zero failures. All existing 232+ tests pass (some with updated construction that no longer includes `r#type`). `cargo clippy` passes with no warnings.

## Constraints

- Do NOT create the full `Message` enum -- only a `#[cfg(test)]` prototype. The full enum is SPEC-052e's scope.
- Do NOT change the wire format semantics -- Rust structs must still produce MsgPack that TS can decode. The integer type changes produce _better_ wire compatibility (matching what TS actually emits), not different semantics.
- Do NOT modify `hlc.rs` -- `Timestamp.millis: u64` and `Timestamp.counter: u32` are already correct.
- Do NOT modify `Cargo.toml` -- no new dependencies required.
- Max 5 files modified (4 planned, within limit).

## Assumptions

- `rmp_serde` supports `#[serde(tag = "type")]` internally-tagged enums with MsgPack named maps. This is the critical assumption validated by the prototype (AC-3). If `rmp_serde` does not support this, the prototype test will fail and SPEC-052e will need an alternative approach (e.g., `#[serde(untagged)]` with manual deserialization).
- TS `msgpackr` encodes integer-valued JS `number`s as MsgPack integers, not floats. This is validated by AC-4 and is well-documented `msgpackr` behavior.
- Removing `r#type: String` from structs does not break any downstream consumer in `core-rust` because no Rust code currently _reads_ these structs' `r#type` field for dispatch (dispatch will be handled by SPEC-052e's enum).
- `Default` for `String` producing `""` is acceptable for structs used primarily in test construction. Production code will construct structs explicitly.

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Prototype `#[serde(tag = "type")]` enum in `mod.rs` `#[cfg(test)]` with 3 variants and round-trip tests. Verify MsgPack wire format for integer types (`u32` vs `f64`). | -- | ~15% |
| G2 | 2 | Refactor `base.rs`: remove `r#type` from 2 structs, change 3 `f64` to integer types, add `Default` to 2 structs, update all tests | G1 | ~15% |
| G3 | 2 | Refactor `sync.rs`: remove `r#type` from 18 structs, change ~12 `f64` to integer types, add `Default` to 2 structs, update all tests | G1 | ~25% |
| G4 | 2 | Refactor `query.rs`: remove `r#type` from 3 structs, add `Default` to 1 struct, update all tests | G1 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3, G4 | Yes | 3 |

**Total workers needed:** 3 (max in any wave)

**Note:** G1 must complete first because it validates the architectural approach (internally-tagged enum support in `rmp_serde`). If the prototype fails, the refactoring approach in G2-G4 would need to be reconsidered. G2, G3, G4 are independent (different files) and can execute in parallel.

## Audit History

### Audit v1 (2026-02-16)
**Status:** APPROVED

**Context Estimate:** ~65% total (sequential), ~25% largest group (G3)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~65% | <=50% | Warning |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~10% (2 waves) | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | <- Estimated (sequential) |
| 70%+ | POOR | - |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | Prototype enum + wire format tests | ~15% | 15% |
| G2 | 2 | Refactor base.rs | ~15% | 30% |
| G3 | 2 | Refactor sync.rs | ~25% | 55% |
| G4 | 2 | Refactor query.rs | ~10% | 65% |

Note: G2, G3, G4 run in parallel (Wave 2), so effective per-worker context is max ~25% (G3), well within the 30% target per worker. Sequential cumulative is 65% but parallel execution keeps each worker in GOOD range.

**Rust Auditor Checklist:**

- [x] No `f64` for integer-semantic fields -- spec fixes all violations
- [x] No `r#type: String` on message structs -- spec removes all 23 instances
- [x] `Default` derived on payload structs with 2+ optional fields -- spec adds to 5 structs
- [x] Enums used for known value sets -- no new string-typed known-value fields in scope
- [x] Wire compatibility: uses `rmp_serde::to_vec_named()` -- confirmed in source
- [x] `#[serde(rename_all = "camelCase")]` on every struct -- existing, preserved
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` -- existing, preserved

**Strategic fit:** Aligned with project goals. Directly implements "Fix-on-port, don't copy bugs" and "No JS-isms in Rust" principles. P0 priority is correct as it blocks 3 downstream specs (052c/d/e).

**Project compliance:** Honors PROJECT.md decisions. Fixes violations of Rust Type Mapping Rules. No new dependencies. File count within Language Profile limit (4/5).

**Language profile:** Compliant with Rust profile. G1 validates approach before G2-G4 implementation.

**Recommendations:**

1. The Scope of Impact table in Context mentions `SyncInitMessage` as a `Default` candidate but R4 correctly excludes it (only 1 optional field after type removal). Consider removing `SyncInitMessage` from the Scope of Impact cell to avoid confusion during implementation. (Minor documentation inconsistency, not blocking.)

2. For `OpRejectedPayload`, R4 lists "1 optional field" as justification, but the PROJECT.md rule says "2+ optional fields". `OpRejectedPayload` has `op_id: String` (required), `reason: String` (required), and `code: Option<u32>` (optional) -- that is only 1 optional field. Consider removing it from the Default list to strictly follow the project convention, or note this as an intentional deviation for consistency with other payload structs. (Minor, not blocking.)

3. The `ClientOp` Default derive produces semantically invalid defaults (`map_name: ""`, `key: ""`). The spec acknowledges this is for test convenience. Consider adding a doc comment `/// Note: Default produces empty map_name/key -- for test convenience only` on the struct to make this explicit in the code. (Optional.)

**Comment:** Exceptionally well-structured specification. Every struct and field change is explicitly enumerated with rationale. The prototype-first approach (G1) correctly gates the architectural validation before mechanical refactoring (G2-G4). Source code verification confirms all 23 `r#type` fields and all `f64` fields match the spec's claims exactly. The 232 test count is precise. Ready for implementation.

### Response v1 (2026-02-16)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [✓] Remove `SyncInitMessage` from Scope of Impact Default candidates -- Verified already removed by user; table now shows only `OpBatchPayload`, `OpAckPayload` in sync.rs row
2. [✓] Remove `OpRejectedPayload` from R4 Default list -- Removed from table, reduced count from 6 to 5 structs, updated Files to Modify (sync.rs: "add Default to 2 structs" instead of 3), updated AC-default-derives description, updated Rust Auditor Checklist (5 structs not 6)
3. [✓] Add doc comment note for `ClientOp` Default semantics -- Added to R4 Note section and Files to Modify description for base.rs

**Skipped:** None

### Audit v2 (2026-02-17)
**Status:** APPROVED

**Context Estimate:** ~65% total (sequential), ~25% largest group (G3)

**Source code verification (fresh audit):**
- `r#type: String` fields: 23 confirmed (2 in base.rs, 18 in sync.rs, 3 in query.rs) -- matches spec
- `f64` fields to fix: 11 scalar + 3 `HashMap<String, f64>` = 14 total -- matches spec
- Total `#[test]` functions: 232 across 9 files -- matches spec
- All Audit v1 recommendations verified as applied in Response v1

**Rust Auditor Checklist:**

- [x] No `f64` for integer-semantic fields -- spec fixes all 14 instances (11 scalar + 3 HashMap)
- [x] No `r#type: String` on message structs -- spec removes all 23 instances
- [x] `Default` derived on payload structs with 2+ optional fields -- spec adds to 5 structs
- [x] Enums used for known value sets -- no new string-typed known-value fields in scope
- [x] Wire compatibility: uses `rmp_serde::to_vec_named()` -- confirmed in source (Cargo.toml)
- [x] `#[serde(rename_all = "camelCase")]` on every struct -- existing, preserved
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` -- existing, preserved

**Goal-Backward Validation:**

| Check | Status |
|-------|--------|
| Truth 1 (no r#type) covered by R2 | Passed |
| Truth 2 (prototype roundtrip) covered by R3 | Passed |
| Truth 3 (hash u32) covered by R1 | Passed |
| Truth 4 (count/code u32) covered by R1 | Passed |
| Truth 5 (timestamp u64) covered by R1 | Passed |
| Truth 6 (Default derives) covered by R4 | Passed |
| Truth 7 (all tests pass) covered by AC-6 | Passed |
| Key links (test updates, literal updates, prototype feasibility) identified | Passed |

**Strategic fit:** Aligned with project goals. Directly implements "Fix-on-port, don't copy bugs" and "No JS-isms in Rust" principles from PROJECT.md.

**Project compliance:** Honors all PROJECT.md decisions. No new dependencies. File count 4/5 within Language Profile limit.

**Language profile:** Compliant with Rust profile. G1 validates approach before G2-G4 implementation (prototype-first, adapted from trait-first for refactor specs).

**Comment:** Re-audited with fresh context after Audit v1 recommendations were applied. All 3 recommendations are correctly incorporated. Source code counts verified independently: 23 r#type fields, 14 f64 fields (including HashMap values), 232 tests. No critical issues. No new recommendations. Ready for implementation via `/sf:run --parallel`.

## Execution Summary

**Executed:** 2026-02-17
**Mode:** orchestrated (sequential fallback -- subagent spawning unavailable)
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3, G4 | complete |

### Files Modified

- `packages/core-rust/src/messages/mod.rs` -- added `#[cfg(test)]` prototype `MessagePrototype` enum with 3 variants and 9 tests (round-trip, discriminator verification, integer wire format)
- `packages/core-rust/src/messages/base.rs` -- removed `r#type` from 2 structs, changed 3 `f64` to integer types, added `Default` to `ClientOp` and `Query`, updated 28 tests
- `packages/core-rust/src/messages/sync.rs` -- removed `r#type` from 18 structs, changed ~12 `f64` to integer types, added `Default` to `OpBatchPayload` and `OpAckPayload`, updated 27 tests
- `packages/core-rust/src/messages/query.rs` -- removed `r#type` from 3 structs, added `Default` to `QueryRespPayload`, updated 9 tests

### Acceptance Criteria Status

- [x] **AC-type-removed:** No struct in base.rs, sync.rs, or query.rs contains `r#type` or `type` field (verified by grep)
- [x] **AC-integer-types:** All fields use specified integer types: `root_hash: u32`, `buckets: HashMap<String, u32>`, `count: u32`, `code: Option<u32>`, `timeout: Option<u64>`, `last_sync_timestamp: Option<u64>`, `protocol_version: Option<u32>`, `limit: Option<u32>`
- [x] **AC-prototype-roundtrip:** `#[cfg(test)]` prototype `#[serde(tag = "type")]` enum with 3 variants round-trips through `rmp_serde` (6 tests)
- [x] **AC-integer-wire-compat:** Test verifies `u32`/`u64` produce MsgPack Integer format, not Float64 (3 tests)
- [x] **AC-default-derives:** 5 structs derive `Default` (`ClientOp`, `Query`, `OpBatchPayload`, `OpAckPayload`, `QueryRespPayload`) -- verified by tests
- [x] **AC-all-tests-pass:** `cargo test` passes with 246 tests (232 original + 14 new), zero failures. Clippy passes with zero warnings on modified files.

### Deviations

- Execution used sequential fallback mode instead of parallel subagent workers (CLI subagent spawning blocked by permissions in current environment). All groups completed successfully regardless.
- `sync_init_camel_case_field_names` test no longer asserts `type` key exists (since `r#type` was removed from `SyncInitMessage`). The test now verifies remaining camelCase fields.

---

## Review History

### Review v1 (2026-02-17)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] **AC-type-removed:** Verified by grep. No struct field definitions for `r#type` or `type` exist in `base.rs`, `sync.rs`, or `query.rs`. The only occurrences of the string `"type"` in `base.rs` are in test code: a predicate attribute value (`Some("type".to_string())`) and a verification test that asserts no `type` key is serialized.
- [x] **AC-integer-types:** All 14 field conversions verified in source:
  - `root_hash: u32` (sync.rs:85, 219, 237)
  - `buckets: HashMap<String, u32>` (sync.rs:112, 265)
  - `bucket_hashes: HashMap<String, u32>` (sync.rs:221)
  - `count: u32` (sync.rs:485)
  - `code: Option<u32>` (sync.rs:458)
  - `timeout: Option<u64>` (base.rs:170, sync.rs:44)
  - `last_sync_timestamp: Option<u64>` (sync.rs:73, 224)
  - `protocol_version: Option<u32>` (base.rs:181)
  - `limit: Option<u32>` (base.rs:129)
  - Zero `f64` fields remain in the messages directory.
- [x] **AC-prototype-roundtrip:** `MessagePrototype` enum in `mod.rs` with 3 variants (OpAck payload-wrapped, SyncInit flat, Batch flat+binary). 6 round-trip tests: `prototype_roundtrip_payload_wrapped`, `prototype_roundtrip_flat`, `prototype_roundtrip_flat_no_optional`, `prototype_roundtrip_flat_with_binary`, `prototype_serialized_contains_type_discriminator` (verifies exactly 1 `type` key per variant with correct discriminator string), `prototype_dispatch_from_msgpack_map`.
- [x] **AC-integer-wire-compat:** 3 wire format tests: `u32_serializes_as_msgpack_integer_not_float` (large u32 + u64 timestamp), `u64_serializes_as_msgpack_integer_not_float` (u64::MAX), `u32_zero_serializes_as_msgpack_integer` (edge case). All assert `rmpv::Value::Integer`, not `F64`/`F32`.
- [x] **AC-default-derives:** 5 structs derive `Default` verified in source: `Query` (base.rs:117), `ClientOp` (base.rs:139), `OpBatchPayload` (sync.rs:34), `OpAckPayload` (sync.rs:422), `QueryRespPayload` (query.rs:102). Test coverage: `query_default_constructs_all_none`, `client_op_default_constructs_with_empty_required_fields`, `op_batch_payload_default_constructs`, `op_ack_payload_default_constructs`, `query_resp_payload_default_constructs`.
- [x] **AC-all-tests-pass:** `cargo test` passes with 246 tests (232 original + 14 new), zero failures. `cargo clippy -- -D warnings` passes with zero warnings.
- [x] **Constraints honored:** `hlc.rs` not modified. `Cargo.toml` not modified. Only 4 files changed (within 5-file limit). Prototype enum is `#[cfg(test)]` only, not a full `Message` enum.
- [x] **Doc comment for ClientOp Default:** Present at base.rs:138 -- `/// Note: \`Default\` produces empty \`map_name\`/\`key\` -- for test convenience only.`
- [x] **serde annotations preserved:** All structs have `#[serde(rename_all = "camelCase")]`. All `Option<T>` fields have `#[serde(skip_serializing_if = "Option::is_none", default)]`. Double-option fields use `deserialize_with = "deserialize_double_option"`.
- [x] **Code quality:** Clean, well-documented Rust. Doc comments on all public types. No `.unwrap()` in production code. No unnecessary `.clone()`. No `unsafe` blocks. Consistent style across all 4 files.
- [x] **Architecture alignment:** Implementation follows PROJECT.md Rust Type Mapping Rules and Auditor Checklist. Enums used for known value sets (WriteConcern, ChangeEventType, PredicateOp, SortDirection, CursorStatus). Wire format uses `rmp_serde::to_vec_named()`.
- [x] **No duplication:** `roundtrip_named` helper defined once per test module. `IntegerWireTest` helper struct defined only in prototype test module. No copy-paste between files.
- [x] **Cognitive load:** Struct definitions are straightforward data types with serde annotations. Test organization follows clear sections (round-trip, camelCase verification, optional field omission, Default verification). Easy to navigate.

**Summary:** Implementation is clean, complete, and fully compliant with the specification. All 6 acceptance criteria are met. All constraints are honored. Code quality is high with thorough test coverage (246 tests, zero failures, zero clippy warnings). The prototype tagged enum successfully validates the architectural approach for SPEC-052e. No issues found.

---

## Completion

**Completed:** 2026-02-17
**Total Commits:** 4
**Audit Cycles:** 2
**Review Cycles:** 1

---
*Created for TODO-079 (Fix-on-Port). Blocks SPEC-052c, SPEC-052d, SPEC-052e.*
