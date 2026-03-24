---
id: SPEC-128
type: feature
status: done
priority: P1
complexity: medium
created: 2026-03-19
source: TODO-128
depends_on: SPEC-127
---

# Wire SchemaService into CrdtService Write Path

## Context

SPEC-127 delivered `SchemaService` (implementing `SchemaProvider`) with `validate_value` in core-rust and schema storage in server-rust. CrdtService already validates writes via `WriteValidator` (auth, ACL, size checks) before CRDT merge. Schema validation must be added as an additional gate in the same pre-merge validation pipeline.

The records in `ClientOp` use `rmpv::Value` (MsgPack dynamic values), but `validate_value` expects `topgun_core::Value`. A conversion function is needed to bridge these types.

## Goal

After this spec, any `ClientOp` or `OpBatch` write to a map with a registered schema is validated against that schema before CRDT merge. Invalid data returns an error to the client -- no merge, no broadcast. Maps without a registered schema pass through unchanged (optional mode).

### Observable Truths

1. A PUT with data violating a registered schema is rejected with an error containing field-level details.
2. A PUT with conforming data to a schema-registered map succeeds normally.
3. A PUT to a map with no registered schema succeeds without validation (passthrough).
4. An OpBatch with one invalid op rejects the entire batch atomically (no ops applied).
5. REMOVE and OR_REMOVE operations skip schema validation (no value to validate).
6. Internal/system calls (no `connection_id`) skip schema validation.

## Task

1. Add `From<rmpv::Value> for Value` conversion in `core-rust/src/types.rs`
2. Add `SchemaInvalid` variant to `OperationError` enum
3. Add `Arc<dyn SchemaProvider>` dependency to `CrdtService`
4. Insert schema validation calls in `handle_client_op` and `handle_op_batch` after `WriteValidator` checks, before CRDT merge
5. Wire `SchemaService` into `CrdtService::new()` at all construction sites

## Requirements

### Files to Modify

**Note:** This spec touches 6 files, one over the Rust profile maximum of 5. The 6th file (`load_harness/main.rs`) requires only a mechanical constructor argument addition and is included to prevent a compile failure rather than omitted.

1. **`packages/core-rust/src/types.rs`** -- Add `impl From<rmpv::Value> for Value` conversion with the following mappings:
   - `rmpv::Value::Nil` → `Value::Null`
   - `rmpv::Value::Boolean(b)` → `Value::Bool(b)`
   - `rmpv::Value::Integer(i)` → `Value::Int(i.as_i64().unwrap_or_else(|| i.as_u64().unwrap_or(u64::MAX) as i64))` — values that fit in `i64` are lossless; values exceeding `i64::MAX` are cast via `as i64` (wrapping). This is acceptable for schema validation because schema integer constraints operate on reasonable ranges.
   - `rmpv::Value::F32(f)` → `Value::Float(f as f64)`
   - `rmpv::Value::F64(f)` → `Value::Float(f)`
   - `rmpv::Value::String(s)` → `Value::String(s.into_str().unwrap_or_default().to_owned())`
   - `rmpv::Value::Binary(b)` → `Value::Bytes(b)`
   - `rmpv::Value::Array(a)` → `Value::Array(a.into_iter().map(Value::from).collect())`
   - `rmpv::Value::Map(m)` → `Value::Map(m.into_iter().map(|(k, v)| (format!("{k}"), Value::from(v))).collect())` — non-string keys converted via `Display`
   - `rmpv::Value::Ext(_, data)` → `Value::Bytes(data)` — MsgPack extension types map to bytes; the type tag is discarded since `Value` has no extension variant

2. **`packages/server-rust/src/service/operation.rs`** -- Add `OperationError::SchemaInvalid { map_name: String, errors: Vec<String> }` variant with `#[error("schema validation failed for map '{map_name}': {}", errors.join("; "))]`. This returns to the client as a distinct error (not 500).

3. **`packages/server-rust/src/service/domain/crdt.rs`** -- Add `schema_provider: Arc<dyn SchemaProvider>` field to `CrdtService`. Add `schema_provider` parameter to `CrdtService::new()`. Add `fn validate_schema_for_op(&self, op: &ClientOp) -> Result<(), OperationError>` helper that:
   - Detects REMOVE using the same logic as `apply_single_op`: `op.op_type == Some("REMOVE") || op.record == Some(None)`. Also skips OR_REMOVE ops. Returns `Ok(())` for all remove variants.
   - Extracts the `rmpv::Value` from `op.record` (LWW) or `op.or_record` (OR). For LWW ops, `LWWRecord.value` is `Option<rmpv::Value>`; if the inner value is `None`, there is no value to validate — return `Ok(())` immediately.
   - Converts to `topgun_core::Value` via `From`
   - Calls `self.schema_provider.validate(map_name, &value)`
   - Returns `Ok(())` on `ValidationResult::Valid`
   - Returns `Err(OperationError::SchemaInvalid { map_name, errors })` on `Invalid`
   - In `handle_client_op`: call after `validate_write`, before `apply_single_op`
   - In `handle_op_batch`: call in the validation loop after `validate_write`, before any apply
   - Update existing test call sites within this file to pass `Arc::new(SchemaService::new())` as the new `schema_provider` argument

4. **`packages/server-rust/src/lib.rs`** -- Update `build_services` (integration_tests::setup) to pass `Arc<dyn SchemaProvider>` (using `Arc::new(SchemaService::new())`) to `CrdtService::new()`.

5. **`packages/server-rust/src/bin/test_server.rs`** -- Wire `SchemaService` into `CrdtService::new()` construction.

6. **`packages/server-rust/benches/load_harness/main.rs`** -- Add `Arc::new(SchemaService::new())` as the `schema_provider` argument to the `CrdtService::new()` call (line ~501). Mechanical change only; no load harness logic changes.

### Interfaces

```rust
// core-rust/src/types.rs
impl From<rmpv::Value> for Value { ... }

// server-rust/src/service/operation.rs (new variant)
OperationError::SchemaInvalid { map_name: String, errors: Vec<String> }

// server-rust/src/service/domain/crdt.rs (updated constructor)
pub fn new(
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    write_validator: Arc<WriteValidator>,
    query_registry: Arc<QueryRegistry>,
    schema_provider: Arc<dyn SchemaProvider>,
) -> Self

// server-rust/src/service/domain/crdt.rs (new helper)
fn validate_schema_for_op(&self, op: &ClientOp) -> Result<(), OperationError>
```

## Acceptance Criteria

1. `From<rmpv::Value> for Value` converts all 10 `rmpv::Value` variants correctly (Nil, Boolean, Integer, F32, F64, String, Binary, Array, Map, Ext). Unit tests cover each variant, including the `Ext` → `Bytes` mapping and the `u64::MAX` integer cast behavior.
2. `OperationError::SchemaInvalid` exists with `map_name` and `errors` fields.
3. `CrdtService::handle_client_op` rejects a PUT with invalid data when schema is registered, returning `SchemaInvalid` error. No CRDT merge occurs, no broadcast.
4. `CrdtService::handle_client_op` allows a PUT with valid data when schema is registered.
5. `CrdtService::handle_client_op` allows any PUT when no schema is registered (passthrough).
6. `CrdtService::handle_op_batch` rejects the entire batch atomically if any op fails schema validation.
7. REMOVE and OR_REMOVE operations bypass schema validation.
8. Internal calls (no `connection_id`) bypass schema validation.
9. All existing CrdtService tests pass with the new `schema_provider` parameter (using `SchemaService::new()` with no schemas registered -- passthrough mode).
10. Cargo test and clippy pass clean.

## Validation Checklist

- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-core 2>&1` -- all tests pass including new `From<rmpv::Value>` tests
- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server 2>&1` -- all tests pass including new schema validation tests in crdt.rs
- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo clippy --release -p topgun-server -p topgun-core 2>&1` -- no warnings
- Send a ClientOp with a required field missing to a schema-registered map -- error returned, store unchanged
- Send a ClientOp with valid data to a schema-registered map -- ack returned, record stored

## Constraints

- Do NOT change the `SchemaProvider` trait (defined in SPEC-127)
- Do NOT change `validate_value` in core-rust (defined in SPEC-127)
- Do NOT add schema validation for server-to-server forwarded ops (trusted origin bypass stays)
- Schema validation runs AFTER `WriteValidator` checks (auth/ACL/size first, then schema)
- `SchemaProvider::validate()` is synchronous (not async) -- no blocking concerns

## Assumptions

- `From<rmpv::Value> for Value` is the right conversion approach (not a fallible `TryFrom`). All `rmpv::Value` variants have a natural mapping to `Value` variants, so the conversion is infallible.
- For LWW records, the value to validate is `op.record -> LWWRecord.value` (the inner `Option<rmpv::Value>`). For OR records, the value to validate is `op.or_record -> ORMapRecord.value`.
- OR_ADD operations with a value should also be schema-validated (the value field of `ORMapRecord`).
- Non-string keys in `rmpv::Value::Map` are converted to strings via `Display` trait (matches existing MsgPack handling patterns in the codebase).
- `rmpv::Value::Ext(_, data)` maps to `Value::Bytes(data)`; the extension type tag is discarded. TopGun does not use MsgPack extension types in CRDT records, so this case is unreachable in practice.
- `rmpv::Integer` values > `i64::MAX` are cast to `i64` via `as i64` (lossy). In practice, schema integer constraints do not operate near `u64::MAX`, so false validation results are not expected.
- The `SchemaInvalid` error is returned to the client as a WebSocket error message (same path as other `OperationError` variants), not a special protocol message.
- REMOVE detection in `validate_schema_for_op` uses the same dual condition as `apply_single_op`: `op.op_type == Some("REMOVE") || op.record == Some(None)`. Both conditions must be checked because a tombstone can be represented either way.
- For LWW ops, `LWWRecord.value` is `Option<rmpv::Value>`. A `None` inner value means the record carries no data (partial tombstone pattern); skip schema validation in this case.

## Goal Analysis

**Goal Statement:** Schema-registered maps enforce data shape on every client write, preventing invalid data from entering the CRDT store.

**Required Artifacts:**
- `From<rmpv::Value> for Value` (core-rust/types.rs) -- bridges wire format to validation format
- `OperationError::SchemaInvalid` (server-rust/operation.rs) -- typed error for client feedback
- Schema validation in CrdtService (server-rust/crdt.rs) -- enforcement point
- Wiring in lib.rs, test_server.rs, and load_harness/main.rs -- dependency injection

**Key Links:**
- `rmpv::Value` -> `Value` conversion is the critical bridge. If lossy, validation produces false positives/negatives.
- `SchemaProvider::validate()` is synchronous and called on the hot path. Already validated as safe in SPEC-127 (DashMap read lock is fast).

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `From<rmpv::Value> for Value` in core-rust/types.rs + unit tests | -- | ~12% |
| G2 | 2 | `OperationError::SchemaInvalid` variant in operation.rs + schema validation in CrdtService (S1: `validate_schema_for_op` + call sites; S2: crdt.rs tests + existing test wiring) | G1 | ~25% |
| G3 | 3 | Wire SchemaService into CrdtService at external construction sites (lib.rs, test_server.rs, load_harness/main.rs) | G2 | ~12% |

**G2 Segments:**
- S1 (~13%): Add `OperationError::SchemaInvalid` in operation.rs; add `schema_provider` field and `validate_schema_for_op` helper to CrdtService; update `handle_client_op` and `handle_op_batch` call sites
- S2 (~12%): Write new schema validation unit tests in crdt.rs; update existing crdt.rs test call sites to pass `Arc::new(SchemaService::new())`

**Total context: ~49%** (G1:12% + G2:25% + G3:12%)

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2 (S1 then S2) | No | 1 |
| 3 | G3 | No | 1 |

**Total workers needed:** 1

## Audit History

### Audit v1 (2026-03-19)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (G1:20% + G2:10% + G3:45% + G4:25%)

**Critical:**
1. **Missing construction site: `benches/load_harness/main.rs`.** There are 10 `CrdtService::new()` call sites across 4 files. The spec lists lib.rs and test_server.rs for wiring but omits `packages/server-rust/benches/load_harness/main.rs` (line 501), which will fail to compile after the constructor signature change. Either add it to Files to Modify or explicitly note it in a constraint (e.g., "load harness is out of scope, will be fixed separately").
2. **File count exceeds Language Profile limit.** The spec already lists 5 files (the Rust profile maximum). Adding the load harness makes 6. The spec must either: (a) split into two specs, or (b) merge G2 (operation.rs -- a 1-line enum variant addition) into the same task as G3 to reduce logical file count, while noting the load harness as a 6th file requiring an acknowledged overage.
3. **G3 estimated at ~45% context -- exceeds 30% per-group target.** crdt.rs is 700+ lines. G3 requires reading the full file, understanding the handler flow, adding `validate_schema_for_op`, modifying two handlers, AND writing new tests. This must be segmented into at least 2 segments (S1: validation logic ~22%, S2: tests ~23%).
4. **Total context ~100% -- POOR quality range.** The sum of all groups (100%) is double the 50% target. The context estimates appear inflated; reassess after restructuring. With realistic estimates (G1:~12%, G2:~5%, G3:~25% with segments, G4:~12%), total is ~54% which is borderline acceptable. Either revise estimates to be realistic or add segmentation.

**Recommendations:**
5. **[Compliance] Goal Analysis "Required Artifacts" omits load harness.** The wiring bullet says "lib.rs and test_server.rs" but should say "lib.rs, test_server.rs, and load_harness/main.rs" (or note it as out of scope).
6. **`rmpv::Value::Ext(i8, Vec<u8>)` handling not specified.** `rmpv::Value` has an `Ext` variant (MsgPack extension type) not covered in the mapping table. Since `From` is infallible, the spec should specify a mapping (e.g., `Ext(_, data)` -> `Value::Bytes(data)` or a wrapper). Without this, the implementer must guess.
7. **`rmpv::Value` integer split.** `rmpv::Value` has both `Integer(rmpv::Integer)` which wraps either `u64` or `i64`. The spec says "integers to `Value::Int`" but `Value::Int(i64)` cannot losslessly represent `u64::MAX`. The spec should clarify truncation behavior (e.g., cast via `as i64` which is lossy for values > `i64::MAX`, or add a `Value::UInt(u64)` variant). For schema validation purposes, overflow is unlikely but should be documented.
8. **G3 and G4 could be merged.** The wiring (G4) is mechanical -- adding one parameter to existing call sites. If G3's context estimate is revised down and segmented, G4's wiring of crdt.rs test call sites could be done as part of G3 (they are in the same file), leaving G4 to handle only lib.rs, test_server.rs, and load_harness. This reduces total waves from 3 to 2.

### Response v1 (2026-03-19)
**Applied:** All critical issues (1-4) and all recommendations (5-8)

**Changes:**
1. [✓] Missing load harness construction site — Added `packages/server-rust/benches/load_harness/main.rs` as file 6 in Files to Modify with a note explaining the acknowledged overage
2. [✓] File count exceeds limit — Added note at top of Files to Modify acknowledging 6-file overage; merged former G2 (operation.rs) into the new combined G2 group alongside crdt.rs to reduce logical group count from 4 to 3; operation.rs is now handled in G2 S1
3. [✓] G3 too large — Former G3 (crdt.rs) is now G2, segmented into S1 (validation logic + operation.rs, ~13%) and S2 (tests + existing test wiring, ~12%)
4. [✓] Context estimates inflated — Revised all estimates: G1:~12%, G2:~25% (S1:13% + S2:12%), G3:~12%; total ~49%
5. [✓] Goal Analysis Required Artifacts omits load harness — Updated wiring bullet to list "lib.rs, test_server.rs, and load_harness/main.rs"
6. [✓] Ext variant unspecified — Added `rmpv::Value::Ext(_, data) → Value::Bytes(data)` to the mapping table in file 1 requirements and to Assumptions
7. [✓] Integer overflow unspecified — Added explicit cast rule (`as i64` for values > `i64::MAX`) to the Integer mapping in file 1 requirements and to Assumptions; updated AC 1 to cover this
8. [✓] Merge G3+G4 — Existing crdt.rs test wiring moved into G2 S2; external wiring (lib.rs, test_server.ts, load_harness) consolidated into new G3

### Audit v2 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~49% total (G1:12% + G2:25% + G3:12%)

**Execution Scope Check:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~49% | <=50% | OK |
| Largest task group | ~25% (G2) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (reject invalid) has artifacts | OK | SchemaInvalid error + validation in CrdtService |
| Truth 2 (accept valid) has artifacts | OK | validation passthrough on Valid result |
| Truth 3 (no schema passthrough) has artifacts | OK | SchemaService returns Valid when no schema |
| Truth 4 (atomic batch) has artifacts | OK | validation loop before any apply |
| Truth 5 (skip REMOVE) has artifacts | OK | validate_schema_for_op skips |
| Truth 6 (skip internal) has artifacts | OK | connection_id gate in handlers |
| From conversion artifact has purpose | OK | enables truths 1-4 |
| SchemaInvalid error has purpose | OK | enables truth 1 |
| CrdtService wiring has purpose | OK | enables all truths |
| Key link: rmpv->Value bridge | OK | conversion specified for all 10 variants |
| Key link: hot-path perf | OK | sync validate, DashMap read lock |

**Assumptions Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Infallible From (not TryFrom) | All rmpv variants mapped; verified against rmpv 1.x API |
| A2 | LWWRecord.value is Option<rmpv::Value> | Verified: hlc.rs line 463 |
| A3 | ORMapRecord.value is rmpv::Value (not Option) | Verified: hlc.rs line 484 |
| A4 | u64>i64::MAX cast is acceptable | Low risk; schema constraints use reasonable ranges |
| A5 | SchemaInvalid uses same error path as other OperationError variants | Verified: WebSocket handler maps OperationError to error messages |

**Project Compliance:** Honors PROJECT.md decisions. No new dependencies. Follows Arc injection pattern. Rust type mapping rules satisfied (no f64 for integers, no r#type fields).

**Language Profile:** 6 files with acknowledged overage for mechanical change. G1 is types-only (trait-first compliant). Acceptable.

**Strategic Fit:** Aligned with project goals -- schema enforcement is a prerequisite for the data platform vision.

**Recommendations:**
1. **[Minor] LWW tombstone detection in helper.** The `validate_schema_for_op` helper must detect REMOVE via the same logic as `apply_single_op` (`op.op_type == Some("REMOVE") || op.record == Some(None)`), not just `op.op_type`. The existing code in crdt.rs lines 278-279 shows the pattern. The implementer will see this, but a brief note in the helper spec (e.g., "use same is_remove logic as apply_single_op") would remove ambiguity.
2. **[Minor] LWW record with value=None.** For LWW PUTs where `op.record = Some(Some(LWWRecord { value: None, .. }))`, the record is a tombstone but `is_remove` is false (only `Some(None)` triggers it). The `validate_schema_for_op` helper should skip validation when the extracted `rmpv::Value` is `None` (the `Option<V>` inner). The spec implicitly handles this via "Extracts the rmpv::Value from op.record" but could note that `LWWRecord.value: Option<rmpv::Value>` means `None` = no value to validate = skip.

**Comment:** Well-structured spec with thorough revision addressing all v1 issues. All 10 audit dimensions pass. Observable truths are complete and testable. The conversion mapping is exhaustive with edge cases documented. Context estimate of ~49% is realistic and within the GOOD quality range. Ready for implementation.

### Response v2 (2026-03-19)
**Applied:** Both recommendations from Audit v2 (items 1 and 2)

**Changes:**
1. [✓] LWW tombstone detection in helper — Replaced the single bullet "Skips REMOVE and OR_REMOVE ops" in File 3 helper spec with an explicit two-condition rule mirroring `apply_single_op`: detect REMOVE via `op.op_type == Some("REMOVE") || op.record == Some(None)`, plus OR_REMOVE. Added corresponding assumption in the Assumptions section.
2. [✓] LWW record with value=None — Expanded the "Extracts the rmpv::Value" bullet in File 3 to explicitly state that `LWWRecord.value` is `Option<rmpv::Value>` and that a `None` inner value means skip validation. Added corresponding assumption in the Assumptions section.

### Audit v3 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~49% total (G1:12% + G2:25% + G3:12%)

**Comment:** All v2 recommendations have been incorporated. The REMOVE detection logic in File 3 now explicitly mirrors `apply_single_op` with the dual condition (`op.op_type == Some("REMOVE") || op.record == Some(None)`). The LWW `value: None` skip case is explicitly documented. Spec is complete, clear, and ready for implementation. No further issues found.

---

## Execution Summary

**Executed:** 2026-03-19
**Commits:** 4

### Files Created
None.

### Files Modified
- `packages/core-rust/src/types.rs` — Added `impl From<rmpv::Value> for Value` with all 10 variant mappings and 14 unit tests
- `packages/server-rust/src/service/operation.rs` — Added `OperationError::SchemaInvalid { map_name, errors }` variant
- `packages/server-rust/src/service/middleware/metrics.rs` — Added `SchemaInvalid` arm to error_kind match (Rule 3: blocking compile)
- `packages/server-rust/src/service/domain/crdt.rs` — Added `schema_provider` field, updated constructor, added `validate_schema_for_op`, wired into handlers, updated all test call sites, added 8 new schema validation tests
- `packages/server-rust/src/lib.rs` — Updated 2 `CrdtService::new` call sites in integration tests
- `packages/server-rust/src/bin/test_server.rs` — Wired `SchemaService` into `CrdtService::new` in `build_services()`
- `packages/server-rust/benches/load_harness/main.rs` — Wired `SchemaService` into `CrdtService::new` in `build_services()`

### Files Deleted
None.

### Acceptance Criteria Status
- [x] 1. `From<rmpv::Value> for Value` converts all 10 variants with unit tests including Ext->Bytes and u64::MAX cast
- [x] 2. `OperationError::SchemaInvalid` exists with `map_name` and `errors` fields
- [x] 3. `CrdtService::handle_client_op` rejects invalid PUT with SchemaInvalid (no merge, no broadcast)
- [x] 4. `CrdtService::handle_client_op` allows PUT with valid data on schema-registered map
- [x] 5. `CrdtService::handle_client_op` allows any PUT when no schema registered (passthrough)
- [x] 6. `CrdtService::handle_op_batch` rejects entire batch atomically if any op fails schema validation
- [x] 7. REMOVE and OR_REMOVE bypass schema validation
- [x] 8. Internal calls (no connection_id) bypass schema validation
- [x] 9. All existing CrdtService tests pass with new `schema_provider` parameter
- [x] 10. Cargo test and clippy pass clean (559 server + 454 core tests, 0 failures)

### Deviations
1. [Rule 3 - Blocking] Added `SchemaInvalid` arm to `service/middleware/metrics.rs` error_kind match — required to prevent non-exhaustive pattern compile error when adding the new variant to `OperationError`.
2. [Rule 1 - Bug] Fixed test helper `make_schema_service()` to return a `ConnectionId` — schema validation bypasses internal calls (no connection_id), so tests validating schema rejection must use a real connection. The test design required this correction to actually exercise the validation path.

### Notes
- The existing `rmpv_to_value()` function in crdt.rs was kept as-is (used internally for storage conversion). The new `From<rmpv::Value>` impl in core-rust is the canonical bridge for schema validation. The two differ only in that `rmpv_to_value` maps `Nil|Ext` to `Null`, while the new `From` impl correctly maps `Nil → Null` and `Ext(_, data) → Bytes(data)` per spec.
- Test count: 559 server-rust (up from 551) + 454 core-rust (up from 440) + 14 new From conversion tests = 1027 total.

---

## Review History

### Review v1 (2026-03-19)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `From<rmpv::Value> for Value` implements all 10 variants (Nil, Boolean, Integer, F32, F64, String, Binary, Array, Map, Ext) with 14 unit tests including Ext->Bytes and u64::MAX cast behavior
- [✓] AC2: `OperationError::SchemaInvalid { map_name: String, errors: Vec<String> }` added with correct `#[error(...)]` format using `errors.join("; ")`
- [✓] AC3: `handle_client_op` rejects invalid PUT returning `SchemaInvalid` — validation occurs before `apply_single_op` so no CRDT merge occurs
- [✓] AC4: `handle_client_op` allows valid PUT on schema-registered map (test `schema_valid_put_succeeds`)
- [✓] AC5: `handle_client_op` allows any PUT when no schema registered — `SchemaService` returns `Valid` for unregistered maps (test `schema_no_schema_registered_passes_through`)
- [✓] AC6: `handle_op_batch` validation loop runs all ops before any apply — atomic rejection verified (test `schema_op_batch_atomic_rejection_on_schema_failure`)
- [✓] AC7: REMOVE (tombstone via `record == Some(None)`) and op_type REMOVE both bypass schema validation; OR_REMOVE bypasses via `is_or_remove` check. Two separate tests cover both REMOVE paths.
- [✓] AC8: Internal calls (no `connection_id`) bypass schema validation entirely — gate is `if let Some(conn_id) = ctx.connection_id` in both handlers
- [✓] AC9: All existing CrdtService tests pass with new `schema_provider` parameter — all test construction sites pass `Arc::new(SchemaService::new())` for passthrough mode
- [✓] AC10: 559 server-rust + 454 core-rust tests pass, clippy clean (verified by running validation commands)
- [✓] Constraint: `SchemaProvider` trait unchanged from SPEC-127; `validate()` remains synchronous
- [✓] Constraint: Schema validation runs after `WriteValidator` (auth/ACL/size) in both handlers
- [✓] Constraint: No server-to-server validation bypass needed — internal call bypass (`connection_id = None`) handles this
- [✓] Deviation 1 (Blocking): `SchemaInvalid` arm added to `metrics.rs` error_kind match — correct and necessary to prevent non-exhaustive pattern compile error
- [✓] Deviation 2 (Bug fix): `make_schema_service()` test helper returns `ConnectionId` — schema validation requires a real connection to be exercised; internal calls bypass validation by design
- [✓] Note on `rmpv_to_value()`: correctly kept as-is (maps Ext→Null for storage); new `From<rmpv::Value>` impl maps Ext→Bytes per spec for validation
- [✓] String conversion: `s.into_str().unwrap_or_default().clone()` is correct — `rmpv::Utf8String::into_str()` returns `Option<String>`, so `.clone()` on `String` is equivalent to `.to_owned()`
- [✓] Architecture: `Arc<dyn SchemaProvider>` injection follows existing dependency injection pattern in `CrdtService`
- [✓] All 6 external `CrdtService::new()` call sites updated (lib.rs x2, test_server.rs, load_harness/main.rs, crdt.rs test helpers x4)

**Summary:** Implementation is complete, correct, and fully compliant with the specification. All 10 acceptance criteria are met. The deviation for `metrics.rs` is a necessary mechanical fix for a non-exhaustive pattern match. The test design deviation (returning `ConnectionId` from `make_schema_service()`) is a correct implementation decision — it properly exercises the schema validation path that internal calls bypass by design. No issues found.

---

## Completion

**Completed:** 2026-03-19
**Total Commits:** 4
**Review Cycles:** 1

### Outcome

Schema-registered maps now enforce data shape on every client write via `SchemaProvider` validation in the CrdtService write path. Invalid data is rejected before CRDT merge with field-level error details.

### Key Files

- `packages/core-rust/src/types.rs` — `From<rmpv::Value> for Value` bridge (10 variant mappings)
- `packages/server-rust/src/service/domain/crdt.rs` — `validate_schema_for_op` enforcement point in write path
- `packages/server-rust/src/service/operation.rs` — `OperationError::SchemaInvalid` error variant

### Patterns Established

None — followed existing `Arc<dyn Trait>` dependency injection and write-path validation patterns.

### Deviations

1. Added `SchemaInvalid` arm to `metrics.rs` error_kind match (compile fix).
2. Test helper returns `ConnectionId` to exercise validation path (internal calls bypass by design).
