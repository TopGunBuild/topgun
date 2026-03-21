---
id: SPEC-136a
type: feature
status: done
priority: P1
complexity: small
parent: SPEC-136
depends_on: []
created: 2026-03-21
source: TODO-070
---

# Shapes: Types, Wire Messages, Operation Variants

## Context

TopGun is implementing partial replication / shapes (SPEC-136) to allow clients to subscribe to filtered subsets of map data. This sub-spec establishes the foundational types, wire messages, and operation variants that all subsequent shape sub-specs build upon.

The Schema System (TODO-069) is complete, providing placeholder `SyncShape` and `Predicate` structs in `core-rust/src/schema.rs`. The `PredicateNode` tree and `evaluate_predicate()` function are available from the DataFusion SQL work (SPEC-135a/b/c). This sub-spec replaces the placeholders with full types using `PredicateNode` directly, and defines the wire protocol for shape subscription.

**Audit fix applied:** `Operation::ShapeSyncInit` variant is included here (Wave 1 types) since it is an Operation variant, even though the sync logic is in SPEC-136d.

**Audit note:** Removing the `Predicate` struct will break the `core-rust/src/lib.rs` re-export at line 32. This sub-spec must update that re-export line. The `lib.rs` change is a trivial one-line removal (remove `Predicate` from the `pub use schema::{...}` block) and is treated as a direct consequence of the `schema.rs` change — it does not count as an additional file for Language Profile purposes.

**Audit note:** `ShapeRespPayload.merkle_root_hash` is always present (not `Option`). A value of 0 means "empty tree" (no Merkle tree built yet). This is consistent with the existing `SyncRespRootPayload.root_hash: u32` pattern.

## Task

Replace the placeholder `SyncShape` and `Predicate` types, define shape wire messages, and add `Operation` variants for shape subscription, unsubscription, and sync init.

## Requirements

### R1: Upgrade SyncShape and Predicate types

**File:** `packages/core-rust/src/schema.rs` (modify)

Replace the placeholder `SyncShape` and `Predicate` structs:

- `SyncShape`: `shape_id: String`, `map_name: String`, `filter: Option<PredicateNode>` (reuse existing tree), `fields: Option<Vec<String>>`, `limit: Option<u32>`
- **Breaking change:** `SyncShape.limit` type changes from `Option<usize>` to `Option<u32>` per Rust Type Mapping Rules (page size / limit fields must be `u32`). Any existing code referencing `SyncShape.limit` as `usize` must be updated.
- Remove standalone `Predicate` struct (replaced by direct `PredicateNode` usage in `SyncShape.filter`)
- Add `#[derive(Default)]` to `SyncShape` (3 optional fields satisfy the 2+ optional fields rule)
- Add `#[serde(rename_all = "camelCase")]` to `SyncShape`
- Add `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` field in `SyncShape`

**File:** `packages/core-rust/src/lib.rs` (one-line consequence of schema.rs change — not counted as a separate file)

Remove `Predicate` from the `pub use schema::{...}` re-export block (line 32). Keep `SyncShape` in the re-export. Also update the doc comment on line 12 of `lib.rs` to remove `Predicate` from the module description (currently reads "`MapSchema`, `SyncShape`, `Predicate` for validation and shapes" — update to "`MapSchema`, `SyncShape` for validation and shapes").

### R2: Shape wire messages

**File:** `packages/core-rust/src/messages/shape.rs` (new)

All payload structs derive `#[serde(rename_all = "camelCase")]` and use `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` field.

**Payload structs (5 payload structs; `ShapeRecord` is a helper struct, not a payload struct):**

- `ShapeSubscribePayload`: embeds `pub shape: SyncShape` directly to avoid field drift between the schema type and the wire message
  - Derives `Default` (via `SyncShape`'s `Default` impl)
- `ShapeUnsubscribePayload`: `shape_id: String`
- `ShapeRespPayload`: `shape_id: String`, `records: Vec<ShapeRecord>`, `merkle_root_hash: u32`, `has_more: Option<bool>`
  - Note: `merkle_root_hash` of 0 means "empty tree" (no Merkle tree built yet)
- `ShapeUpdatePayload`: `shape_id: String`, `key: String`, `value: Option<rmpv::Value>` (None = removed from shape), `change_type: ChangeEventType`
- `ShapeSyncInitPayload`: `shape_id: String`, `root_hash: u32` (client's current shape Merkle root)

**Helper struct:**

- `ShapeRecord`: `key: String`, `value: rmpv::Value` — the unit of record transfer in shape responses (not a payload struct; does not wrap a message variant)

**Message wrapper structs** (following `SyncInitMessage` naming convention, each containing `pub payload: XxxPayload` matching the `QuerySubMessage` pattern):

- `ShapeSubscribeMessage`: `pub payload: ShapeSubscribePayload`
- `ShapeUnsubscribeMessage`: `pub payload: ShapeUnsubscribePayload`
- `ShapeRespMessage`: `pub payload: ShapeRespPayload`
- `ShapeUpdateMessage`: `pub payload: ShapeUpdatePayload`
- `ShapeSyncInitMessage`: `pub payload: ShapeSyncInitPayload`

**Merkle sync message strategy -- reuse existing protocol:**

Shape Merkle sync reuses the existing `SyncRespRootMessage`, `SyncRespBucketsMessage`, and `SyncRespLeafMessage` structs. Shape paths are distinguished by prefixing with the `shape_id` (e.g., `"<shape_id>/<partition_id>/<depth>/<bucket>"`). No new `ShapeSyncResp*` message variants are added.

**New `Message` enum variants** in `core-rust/src/messages/mod.rs`, each with an explicit SCREAMING_SNAKE_CASE serde rename tag matching the existing convention:

- `#[serde(rename = "SHAPE_SUBSCRIBE")] ShapeSubscribe(ShapeSubscribeMessage)`
- `#[serde(rename = "SHAPE_UNSUBSCRIBE")] ShapeUnsubscribe(ShapeUnsubscribeMessage)`
- `#[serde(rename = "SHAPE_RESP")] ShapeResp(ShapeRespMessage)`
- `#[serde(rename = "SHAPE_UPDATE")] ShapeUpdate(ShapeUpdateMessage)`
- `#[serde(rename = "SHAPE_SYNC_INIT")] ShapeSyncInit(ShapeSyncInitMessage)`

Add all payload, helper, and wrapper types to the `pub use shape::{...}` re-export block.

Add MsgPack roundtrip and camelCase serialization tests for all message types.

### R3: Operation variants

**File:** `packages/server-rust/src/service/operation.rs` (modify)

Add three new `Operation` enum variants:

- `Operation::ShapeSubscribe` -- client subscribes to a shape
- `Operation::ShapeUnsubscribe` -- client unsubscribes from a shape
- `Operation::ShapeSyncInit` -- client initiates shape-specific Merkle delta sync

**File:** `packages/server-rust/src/service/classify.rs` (modify)

Add classification for the new `Message` variants:

- `Message::ShapeSubscribe` -> `Operation::ShapeSubscribe` with `service_name: service_names::SHAPE`
- `Message::ShapeUnsubscribe` -> `Operation::ShapeUnsubscribe` with `service_name: service_names::SHAPE`
- `Message::ShapeSyncInit` -> `Operation::ShapeSyncInit` with `service_name: service_names::SHAPE`
- `Message::ShapeResp` -> `ClassifyError::ServerToClient` (server-to-client only)
- `Message::ShapeUpdate` -> `ClassifyError::ServerToClient` (server-to-client only)

Add `pub const SHAPE: &str = "shape";` to `service_names` (in the same file or wherever `service_names::SYNC` is defined). The `ShapeService` that handles this `service_name` will be created in SPEC-136c; the `classify.rs` change only needs to produce the correct `OperationContext` — it does not require the service to exist yet.

## Acceptance Criteria

1. `SyncShape` has fields: `shape_id`, `map_name`, `filter: Option<PredicateNode>`, `fields: Option<Vec<String>>`, `limit: Option<u32>`
2. `SyncShape` derives `Default`, `Serialize`, `Deserialize`, with `rename_all = "camelCase"` and `skip_serializing_if`/`default` on all `Option` fields
3. `Predicate` struct is removed from `core-rust/src/schema.rs`
4. `Predicate` is removed from the `core-rust/src/lib.rs` re-export block, and the doc comment on `lib.rs` line 12 is updated to remove `Predicate` from the module description
5. All 5 shape payload structs exist with correct field types and serde annotations (`ShapeRecord` is a helper struct and is not counted among the 5 payload structs)
6. `ShapeSubscribePayload` embeds `pub shape: SyncShape` rather than duplicating `SyncShape`'s fields
7. Each wrapper struct contains `pub payload: XxxPayload` (matching the `QuerySubMessage` / `pub payload: XxxPayload` pattern)
8. All 5 `Message` enum variants exist with explicit SCREAMING_SNAKE_CASE serde rename tags: `"SHAPE_SUBSCRIBE"`, `"SHAPE_UNSUBSCRIBE"`, `"SHAPE_RESP"`, `"SHAPE_UPDATE"`, `"SHAPE_SYNC_INIT"`
9. `Operation::ShapeSubscribe`, `Operation::ShapeUnsubscribe`, and `Operation::ShapeSyncInit` variants exist
10. `classify.rs` routes shape messages to `service_names::SHAPE`; `ShapeResp` and `ShapeUpdate` return `ServerToClient` error
11. `pub const SHAPE: &str = "shape";` exists in `service_names`
12. MsgPack roundtrip tests pass for all shape message types
13. `cargo test --release -p topgun-core` passes (all existing + new tests)
14. `cargo test --release -p topgun-server` passes (all existing tests, no regressions)
15. `ShapeRespPayload.merkle_root_hash` is `u32` (not `Option`), value 0 means empty tree

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-core` -- all tests pass
2. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all tests pass
3. Verify `Predicate` struct no longer exists in `core-rust/src/schema.rs`
4. Verify no compilation errors from removing `Predicate` re-export
5. Verify `lib.rs` doc comment no longer mentions `Predicate`
6. Verify each `Message` variant for shapes has an explicit `#[serde(rename = "SHAPE_...")]` tag

## Constraints

- Do NOT modify the existing full-map sync messages (`SyncInit`, `SyncRespRoot`, etc.)
- Do NOT add any service implementation -- this sub-spec is types/messages only
- Wire messages MUST use `rmp_serde::to_vec_named()` with `#[serde(rename_all = "camelCase")]`
- Max 5 Rust files (`schema.rs`, `messages/shape.rs`, `messages/mod.rs`, `operation.rs`, `classify.rs`; the `lib.rs` one-line change is a direct consequence of `schema.rs` and is handled in the same pass)

## Assumptions

1. Shape ID is client-assigned (UUID v4). Server rejects duplicates.
2. `Predicate` struct in `core-rust/src/schema.rs` can be removed since `SyncShape.filter` uses `PredicateNode` directly. If external code depends on `Predicate`, it will need updating.
3. Field projection includes the record key implicitly -- the key is always present in shape responses regardless of `fields` list.

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Upgrade `SyncShape`, remove `Predicate`, update `lib.rs` re-export and doc comment | -- | ~5% |
| G2 | 2 | Shape payload structs, wrapper structs, `Message` variants, roundtrip tests | G1 | ~10% |
| G3 | 2 | `Operation` variants, `service_names::SHAPE`, `classify.rs` routing | G1 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-21)
**Status:** NEEDS_REVISION

**Context Estimate:** ~25% total

**Critical:**
1. File count (6) exceeds Language Profile limit (5). The spec touches `schema.rs`, `lib.rs`, `messages/shape.rs`, `messages/mod.rs` (core-rust) plus `operation.rs`, `classify.rs` (server-rust). The spec's own Constraints section says "Max 5 Rust files." Resolution: fold the `lib.rs` one-line change into the `schema.rs` requirement (R1) without listing it as a separate file, and update the doc comment on `lib.rs` line 12 in the same pass. Alternatively, remove the separate payload+wrapper pattern and put fields directly in the message wrapper structs (eliminating the need for separate payload structs), which would let `messages/shape.rs` absorb all shape types and the `messages/mod.rs` changes become minimal re-exports only. Either way, the spec must reconcile the 6-file reality with the 5-file constraint.
2. Missing `#[serde(rename = "...")]` tag values for `Message` enum variants. The existing convention uses SCREAMING_SNAKE_CASE (e.g., `"SYNC_INIT"`, `"QUERY_SUB"`). AC6 requires "correct serde rename tags" but the spec body never specifies the actual tag strings. Add explicit rename values, e.g.: `#[serde(rename = "SHAPE_SUBSCRIBE")]`, `#[serde(rename = "SHAPE_UNSUBSCRIBE")]`, `#[serde(rename = "SHAPE_RESP")]`, `#[serde(rename = "SHAPE_UPDATE")]`, `#[serde(rename = "SHAPE_SYNC_INIT")]`.

**Recommendations:**
3. Update the doc comment in `lib.rs` line 12 to remove `Predicate` from the module description: currently reads "`MapSchema`, `SyncShape`, `Predicate` for validation and shapes."
4. R2 lists `ShapeRecord` under "Payload structs" but AC5 says "All 5 shape payload structs" (excluding `ShapeRecord`). Clarify whether `ShapeRecord` is counted as a payload struct or a helper struct.
5. The wrapper struct internal structure is implied but not explicit. Specify that each wrapper contains `pub payload: XxxPayload` (matching the `QuerySubMessage` pattern), or state that fields are inlined (matching the `SyncInitMessage` pattern). Currently "wraps" is ambiguous between these two established conventions.
6. [Strategic] `ShapeSubscribePayload` duplicates all `SyncShape` fields. Consider whether the payload should embed `SyncShape` directly (e.g., `pub shape: SyncShape`) to avoid field drift between the schema type and the wire message. This is a design choice, not a defect.
7. R3 does not specify `service_name` for shape operations. The `OperationContext` requires a `service_name` (e.g., `service_names::SYNC` or a new `service_names::SHAPE`). Clarify which domain service handles shape operations -- presumably sync or a new "shape" service from SPEC-136c.

**Auditor Checklist (Rust):**
- [x] No `f64` for integer-semantic fields (`u32` used for limit, merkle_root_hash, root_hash)
- [x] No `r#type: String` on message structs (enum tag via `#[serde(tag = "type")]`)
- [x] `Default` derived on payload structs with 2+ optional fields (SyncShape, ShapeSubscribePayload)
- [x] Enums used for known value sets (`ChangeEventType`)
- [x] Wire compatibility: `rmp_serde::to_vec_named()` specified in constraints
- [x] `#[serde(rename_all = "camelCase")]` specified for all structs
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` specified for all Option fields

**Strategic fit:** Aligned with project goals -- shapes are identified as "table stakes" in PROJECT.md.
**Project compliance:** Honors PROJECT.md decisions (MsgPack wire format, PredicateNode reuse, u32 for limits). One violation: file count exceeds Language Profile limit (critical issue 1).
**Language profile:** Non-compliant -- file count 6 exceeds Rust profile max of 5 (critical issue 1).

### Response v1 (2026-03-21)
**Applied:** All 7 items (critical 1-2, recommendations 3-7)

**Changes:**
1. [✓] File count exceeds limit — Reclassified `lib.rs` as a one-line consequence of `schema.rs`, not a counted file. Updated Constraints to enumerate the 5 counted files explicitly. Updated R1 and task group G1 descriptions to clarify `lib.rs` is handled in the same pass as `schema.rs`.
2. [✓] Missing serde rename tags — Added explicit SCREAMING_SNAKE_CASE `#[serde(rename = "...")]` tags for all 5 `Message` variants: `"SHAPE_SUBSCRIBE"`, `"SHAPE_UNSUBSCRIBE"`, `"SHAPE_RESP"`, `"SHAPE_UPDATE"`, `"SHAPE_SYNC_INIT"`. Added AC8 to verify. Added validation checklist item 6.
3. [✓] lib.rs doc comment — Added instruction in R1 to update the doc comment on `lib.rs` line 12 to remove `Predicate`. Reflected in AC4 and validation checklist item 5.
4. [✓] ShapeRecord helper vs payload struct — R2 now explicitly labels the 5 payload structs and separates `ShapeRecord` under "Helper struct". AC5 clarifies `ShapeRecord` is not counted among the 5 payload structs.
5. [✓] Wrapper struct pattern ambiguous — R2 now explicitly specifies each wrapper contains `pub payload: XxxPayload` matching the `QuerySubMessage` pattern. Added AC7.
6. [✓] ShapeSubscribePayload duplicates SyncShape fields — Changed `ShapeSubscribePayload` to embed `pub shape: SyncShape` directly. Removed duplicated field list. Added AC6.
7. [✓] Missing service_name for shape operations — Added `pub const SHAPE: &str = "shape";` to R3, classify.rs routing now specifies `service_name: service_names::SHAPE`, and noted that ShapeService will be created in SPEC-136c. Added AC11.

### Audit v2 (2026-03-21 18:45)
**Status:** APPROVED

**Context Estimate:** ~25% total

**Auditor Checklist (Rust):**
- [x] No `f64` for integer-semantic fields (`u32` for limit, merkle_root_hash, root_hash)
- [x] No `r#type: String` on message structs (enum tag via `#[serde(tag = "type")]`)
- [x] `Default` derived on payload structs with 2+ optional fields (SyncShape, ShapeSubscribePayload)
- [x] Enums used for known value sets (`ChangeEventType`)
- [x] Wire compatibility: `rmp_serde::to_vec_named()` specified in constraints
- [x] `#[serde(rename_all = "camelCase")]` specified for all structs
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` specified for all Option fields

**Assumptions verified against codebase:**
- `Predicate` struct is only referenced in `schema.rs` (definition) and `lib.rs` (re-export + doc comment). No other Rust code imports or uses it. Removal is safe.
- `PredicateNode` is already defined in `messages/base.rs` and re-exported from `lib.rs`. The import path for `schema.rs` will be `use crate::messages::base::PredicateNode` (or `crate::PredicateNode`).
- `ChangeEventType` is available from `messages/base.rs` with variants `ENTER`, `UPDATE`, `LEAVE`.
- `SyncRespRootPayload.root_hash` is `u32` (not Option), confirming the `merkle_root_hash: u32` pattern.
- `service_names` module is in `operation.rs` (lines 28-36), not a separate file.
- The `QuerySubMessage` pattern (`pub payload: QuerySubPayload`) is confirmed at `query.rs` lines 57-59.
- The `Message` enum uses newtype variants for messages with wrapper structs (e.g., `QuerySub(QuerySubMessage)`), consistent with the spec's proposed `ShapeSubscribe(ShapeSubscribeMessage)` form.

**Strategic fit:** Aligned with project goals -- shapes are "table stakes" per PROJECT.md.
**Project compliance:** Honors all PROJECT.md decisions (MsgPack wire format, PredicateNode reuse, u32 for limits, camelCase serde).
**Language profile:** Compliant -- 5 counted files (schema.rs, messages/shape.rs, messages/mod.rs, operation.rs, classify.rs); lib.rs correctly treated as a one-line consequence.

**Recommendations:**
1. R3 specifies `Operation` variant names but not the exact field types. The established pattern is `VariantName { ctx: OperationContext, payload: messages::XxxMessage }`. While unambiguous from codebase context, explicitly stating the payload types (e.g., `Operation::ShapeSubscribe { ctx: OperationContext, payload: messages::ShapeSubscribeMessage }`) would make the spec fully self-contained. This does not block implementation.

**Comment:** Well-structured spec with clear requirements, explicit serde annotations, and thorough acceptance criteria. All v1 audit issues were fully resolved. The spec correctly reuses existing patterns (PredicateNode, ChangeEventType, QuerySubMessage wrapper pattern) and avoids duplication. Ready for implementation.

## Execution Summary

**Executed:** 2026-03-21
**Mode:** orchestrated
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |

### Files Created

- `packages/core-rust/src/messages/shape.rs`

### Files Modified

- `packages/core-rust/src/schema.rs`
- `packages/core-rust/src/lib.rs`
- `packages/core-rust/src/messages/mod.rs`
- `packages/server-rust/src/service/operation.rs`
- `packages/server-rust/src/service/classify.rs`

### Acceptance Criteria Status

- [x] `SyncShape` has fields: `shape_id`, `map_name`, `filter: Option<PredicateNode>`, `fields: Option<Vec<String>>`, `limit: Option<u32>`
- [x] `SyncShape` derives `Default`, `PartialEq`, `Serialize`, `Deserialize`, with `rename_all = "camelCase"` and `skip_serializing_if`/`default` on all `Option` fields
- [x] `Predicate` struct is removed from `core-rust/src/schema.rs`
- [x] `Predicate` is removed from the `core-rust/src/lib.rs` re-export block, and the doc comment on `lib.rs` line 12 is updated to remove `Predicate` from the module description
- [x] All 5 shape payload structs exist with correct field types and serde annotations (`ShapeRecord` is a helper struct and is not counted among the 5 payload structs)
- [x] `ShapeSubscribePayload` embeds `pub shape: SyncShape` rather than duplicating `SyncShape`'s fields
- [x] Each wrapper struct contains `pub payload: XxxPayload` (matching the `QuerySubMessage` / `pub payload: XxxPayload` pattern)
- [x] All 5 `Message` enum variants exist with explicit SCREAMING_SNAKE_CASE serde rename tags: `"SHAPE_SUBSCRIBE"`, `"SHAPE_UNSUBSCRIBE"`, `"SHAPE_RESP"`, `"SHAPE_UPDATE"`, `"SHAPE_SYNC_INIT"`
- [x] `Operation::ShapeSubscribe`, `Operation::ShapeUnsubscribe`, and `Operation::ShapeSyncInit` variants exist
- [x] `classify.rs` routes shape messages to `service_names::SHAPE`; `ShapeResp` and `ShapeUpdate` return `ServerToClient` error
- [x] `pub const SHAPE: &str = "shape";` exists in `service_names`
- [x] MsgPack roundtrip tests pass for all shape message types (17 new tests)
- [x] `cargo test --release -p topgun-core` passes (494 tests)
- [x] `cargo test --release -p topgun-server` passes (565 tests, no regressions)
- [x] `ShapeRespPayload.merkle_root_hash` is `u32` (not `Option`), value 0 means empty tree

### Deviations

- `SyncShape` received `PartialEq` derive (not in original spec requirements, but required by message tests that derive `PartialEq` on `ShapeSubscribePayload` which embeds `SyncShape`)

---

## Review History

### Review v1 (2026-03-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `SyncShape` has all 5 required fields with correct types (`shape_id: String`, `map_name: String`, `filter: Option<PredicateNode>`, `fields: Option<Vec<String>>`, `limit: Option<u32>`)
- [✓] AC2: `SyncShape` derives `Default`, `PartialEq`, `Serialize`, `Deserialize` with `rename_all = "camelCase"` and `skip_serializing_if`/`default` on all `Option` fields
- [✓] AC3: `Predicate` struct fully removed from `core-rust/src/schema.rs` — no lingering references anywhere in codebase
- [✓] AC4: `Predicate` removed from `core-rust/src/lib.rs` re-export block; doc comment on line 12 correctly reads "`MapSchema`, `SyncShape` for validation and shapes" (no `Predicate`)
- [✓] AC5: All 5 payload structs exist (`ShapeSubscribePayload`, `ShapeUnsubscribePayload`, `ShapeRespPayload`, `ShapeUpdatePayload`, `ShapeSyncInitPayload`); `ShapeRecord` correctly classified as helper struct
- [✓] AC6: `ShapeSubscribePayload` embeds `pub shape: SyncShape` directly
- [✓] AC7: All 5 wrapper structs contain `pub payload: XxxPayload` following `QuerySubMessage` pattern
- [✓] AC8: All 5 `Message` variants have explicit SCREAMING_SNAKE_CASE rename tags (`"SHAPE_SUBSCRIBE"`, `"SHAPE_UNSUBSCRIBE"`, `"SHAPE_RESP"`, `"SHAPE_UPDATE"`, `"SHAPE_SYNC_INIT"`)
- [✓] AC9: `Operation::ShapeSubscribe`, `Operation::ShapeUnsubscribe`, `Operation::ShapeSyncInit` variants exist with correct `{ ctx: OperationContext, payload: messages::XxxMessage }` structure
- [✓] AC10: `classify.rs` routes all 3 client-to-server shape messages to `service_names::SHAPE`; `ShapeResp` and `ShapeUpdate` return `ClassifyError::ServerToClient`
- [✓] AC11: `pub const SHAPE: &str = "shape";` exists in `service_names` module in `operation.rs`; test `service_name_constants` verifies it
- [✓] AC12: 17 MsgPack roundtrip and camelCase serialization tests pass for all message types
- [✓] AC13: `cargo test --release -p topgun-core` passes — 477 unit tests + 10 integration tests + 7 doc tests (494 total)
- [✓] AC14: `cargo test --release -p topgun-server` passes — 565 tests, 0 failures, no regressions
- [✓] AC15: `ShapeRespPayload.merkle_root_hash` is `u32` (not `Option`); 0 value documented as "empty tree"
- [✓] Build check: `cargo clippy -p topgun-core -p topgun-server -- -D warnings` exits 0, no warnings
- [✓] Constraint compliance: no existing sync messages modified, no service implementation added, `rmp_serde::to_vec_named()` used in tests, file count within 5-file Language Profile limit
- [✓] `PartialEq` deviation on `SyncShape` is appropriate — required for test assertions on `ShapeSubscribePayload` which embeds it; this is an improvement, not a defect
- [✓] No security issues, no hardcoded secrets, no `unwrap()` in production code paths

**Summary:** Implementation is fully compliant with all 15 acceptance criteria. All tests pass (494 core + 565 server), clippy is clean, and the `Predicate` struct has been completely eliminated with no lingering references. The deviation (`PartialEq` on `SyncShape`) is a sensible improvement needed for test ergonomics and does not conflict with any constraint.

---

## Completion

**Completed:** 2026-03-21
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Established the foundational shape types, wire messages (5 payload structs + 5 Message variants), and Operation routing for partial replication. Replaced placeholder `SyncShape`/`Predicate` with production types using `PredicateNode` directly.

### Key Files

- `packages/core-rust/src/messages/shape.rs` — all shape payload, helper, and wrapper structs
- `packages/core-rust/src/schema.rs` — upgraded `SyncShape` with full fields, removed `Predicate`
- `packages/server-rust/src/service/operation.rs` — `Operation::ShapeSubscribe/Unsubscribe/SyncInit` + `service_names::SHAPE`
- `packages/server-rust/src/service/classify.rs` — shape message routing to SHAPE service

### Patterns Established

- Shape wire messages reuse existing Merkle sync protocol (`SyncRespRoot/Buckets/Leaf`) with shape_id prefix — no new sync message variants needed.
- `ShapeSubscribePayload` embeds `SyncShape` directly to avoid field drift between schema and wire types.

### Deviations

- `SyncShape` received `PartialEq` derive (not in spec, but required for test assertions on embedding payload structs).
