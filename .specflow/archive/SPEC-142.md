---
id: SPEC-142
type: feature
status: done
priority: P1
complexity: small
created: 2026-03-24
source: TODO-181
delta: true
---

# Extend QUERY_SUB/QUERY_RESP Wire Messages with Shape Capabilities

## Context

Query and Shape are parallel implementations of the same concept (filtered data subscription with ENTER/UPDATE/LEAVE deltas). Both use identical `PredicateNode` filter syntax but expose different wire messages. TODO-181 plans a 4-step merge. This is step 1/4: extend the existing QUERY_* message family to carry Shape capabilities (`fields` projection, `merkleRootHash`, `hasMore`) and add a `QUERY_SYNC_INIT` message for Merkle delta reconnect.

This step is schema-only. No behavioral changes to server or client logic. Server and client behavior changes happen in TODO-182/183. SHAPE_* messages are NOT removed yet (TODO-184).

## Delta

### MODIFIED
- `packages/core/src/schemas/query-schemas.ts` -- Add optional `fields` to QuerySubPayload, add optional `merkleRootHash` to QueryRespPayload, add QUERY_SYNC_INIT message schema
- `packages/core-rust/src/messages/query.rs` -- Add optional `fields` to QuerySubPayload, add optional `merkle_root_hash` to QueryRespPayload, add QuerySyncInitPayload/QuerySyncInitMessage structs
- `packages/core-rust/src/messages/mod.rs` -- Add `QUERY_SYNC_INIT` variant to Message enum, add re-exports for new types
- `packages/server-rust/src/service/classify.rs` -- Add classify arm for `Message::QuerySyncInit` routing to query service
- `packages/server-rust/src/service/operation.rs` -- Add `QuerySyncInit` variant to Operation enum

## Requirements

### R1: Extend TS QuerySubPayload with `fields`

In `packages/core/src/schemas/query-schemas.ts`, add to the inline `z.object({...})` payload inside `QuerySubMessageSchema` (the payload object is not separately named):

```
fields: z.array(z.string()).optional(),
```

This mirrors `SyncShape.fields` -- an optional list of field names for projection (partial replication).

### R2: Extend TS QueryRespPayload with `merkleRootHash`

In `packages/core/src/schemas/query-schemas.ts`, add to the `QueryRespPayloadSchema` object:

```
merkleRootHash: z.number().int().optional(),
```

The `hasMore` field already exists on `QueryRespPayloadSchema`. No change needed for it.

### R3: Add TS QUERY_SYNC_INIT message

In `packages/core/src/schemas/query-schemas.ts`, add:

- `QuerySyncInitPayloadSchema`: `{ queryId: z.string(), rootHash: z.number().int() }`
- `QuerySyncInitMessageSchema`: `{ type: z.literal('QUERY_SYNC_INIT'), payload: QuerySyncInitPayloadSchema }`
- Export types `QuerySyncInitPayload` and `QuerySyncInitMessage`

Update `packages/core/src/schemas/index.ts`:
- Add `QuerySyncInitMessageSchema` to the import from `'./query-schemas'`
- Add `QuerySyncInitMessageSchema` to the `MessageSchema` discriminated union (in the Query section, after `QueryUpdateMessageSchema`)
- Note: `export * from './query-schemas'` already re-exports all named exports, so no additional re-export lines are needed

### R4: Extend Rust QuerySubPayload with `fields`

In `packages/core-rust/src/messages/query.rs`, add to `QuerySubPayload`:

```rust
#[serde(skip_serializing_if = "Option::is_none", default)]
pub fields: Option<Vec<String>>,
```

### R5: Extend Rust QueryRespPayload with `merkle_root_hash`

In `packages/core-rust/src/messages/query.rs`, add to `QueryRespPayload`:

```rust
#[serde(skip_serializing_if = "Option::is_none", default)]
pub merkle_root_hash: Option<u32>,
```

Use `u32` to match the existing `ShapeRespPayload.merkle_root_hash` and `SyncRespRootPayload.root_hash` types.

### R6: Add Rust QuerySyncInitPayload and QuerySyncInitMessage

In `packages/core-rust/src/messages/query.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySyncInitPayload {
    pub query_id: String,
    pub root_hash: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySyncInitMessage {
    pub payload: QuerySyncInitPayload,
}
```

### R7: Add QUERY_SYNC_INIT to Rust Message enum

In `packages/core-rust/src/messages/mod.rs`:

- Add `#[serde(rename = "QUERY_SYNC_INIT")] QuerySyncInit(QuerySyncInitMessage)` variant to the `Message` enum, in the query domain section (after `QueryResp`)
- Add `QuerySyncInitMessage` and `QuerySyncInitPayload` to the re-exports from `query` module

### R8: Add QuerySyncInit to Operation enum and classify

In `packages/server-rust/src/service/operation.rs`:

- Add `QuerySyncInit { ctx: OperationContext, payload: messages::QuerySyncInitMessage }` variant to `Operation`
- Add match arms in all exhaustive matches (`context()`, `set_connection_id()`, and any other match blocks)

In `packages/server-rust/src/service/classify.rs`:

- Add `Message::QuerySyncInit(payload)` arm that routes to `service_names::QUERY` and produces `Operation::QuerySyncInit`

### R9: Tests

**Rust tests** (in `packages/core-rust/src/messages/query.rs` `mod tests`):

- `query_sub_with_fields_roundtrip`: QuerySubMessage with `fields: Some(vec!["name", "email"])` round-trips correctly
- `query_sub_fields_omitted_when_none`: Serialized bytes do not contain "fields" key when `fields` is `None`
- `query_resp_with_merkle_root_hash_roundtrip`: QueryRespPayload with `merkle_root_hash: Some(12345)` round-trips correctly
- `query_resp_merkle_root_hash_omitted_when_none`: Serialized bytes do not contain "merkleRootHash" key when `merkle_root_hash` is `None`
- `query_sync_init_roundtrip`: QuerySyncInitMessage round-trips correctly
- `query_sync_init_camel_case`: Serialized field names are `queryId` and `rootHash`

**Rust classify test** (in `packages/server-rust/src/service/classify.rs` `mod tests`):

- `classify_query_sync_init_routes_to_query`: Verify `Message::QuerySyncInit` classifies to `Operation::QuerySyncInit` with `service_names::QUERY`

## Acceptance Criteria

1. `QuerySubPayload` in both TS and Rust accepts optional `fields: string[]` / `fields: Option<Vec<String>>` -- existing messages without `fields` deserialize unchanged
2. `QueryRespPayload` in both TS and Rust accepts optional `merkleRootHash` / `merkle_root_hash: Option<u32>` -- existing messages without it deserialize unchanged
3. `QUERY_SYNC_INIT` message type exists in both TS Zod schema and Rust Message enum with payload `{ queryId, rootHash }`
4. `QUERY_SYNC_INIT` routes to the query service in `classify.rs`
5. All new optional fields use `skip_serializing_if = "Option::is_none"` in Rust and `.optional()` in Zod
6. All existing tests pass without modification (backward compatible additions)
7. New round-trip and camelCase tests pass for all added fields and messages

## Constraints

- Do NOT add any behavioral logic -- this is schema extension only
- Do NOT remove or modify SHAPE_* messages (that is TODO-184)
- Do NOT change QueryHandle, ShapeHandle, SyncEngine, or any client/server logic
- Use `u32` for `merkle_root_hash` and `root_hash` to match existing codebase convention (not `u64`)
- The new `QUERY_SYNC_INIT` handler in the query domain service should be a no-op stub or not implemented at all -- just the routing plumbing in classify/operation

## Assumptions

- `merkle_root_hash` uses `u32` (matching existing `ShapeRespPayload.merkle_root_hash` and `SyncRespRootPayload.root_hash`) rather than `u64` as mentioned in the task description. The task description likely inherited `u64` from PROJECT.md integer type mapping guidance, but the semantic type for Merkle root hashes in this codebase is consistently `u32`.
- `QuerySyncInitPayload` uses `queryId` + `rootHash` field names (paralleling `ShapeSyncInitPayload` which uses `shapeId` + `rootHash`).
- The `hasMore` field on `QueryRespPayload` already exists (confirmed by reading query-schemas.ts and query.rs), so no addition needed for it.
- No changes to `packages/core/src/schemas/base-schemas.ts` -- `PredicateNode` is already shared and sufficient.
- The query domain service does not need a handler implementation for `QUERY_SYNC_INIT` in this spec -- only the wire schema and routing classification.

## Audit History

### Audit v1 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields — `u32` used for hash values, correct
- [x] No `r#type: String` on message structs — enum owns the tag
- [x] `Default` derived on payload structs with 2+ optional fields — QueryRespPayload already has Default
- [x] Enums used for known value sets — N/A (no enum fields added)
- [x] Wire compatibility — existing codebase uses `to_vec_named()`
- [x] `#[serde(rename_all = "camelCase")]` on every struct — present in R6
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` — present in R4, R5

**Delta validation:** 5/5 entries valid (all MODIFIED files exist)

**Strategic fit:** ✓ Aligned with project goals (Query/Shape merge step 1/4)
**Project compliance:** ✓ Honors PROJECT.md decisions
**Language profile:** ✓ Compliant with Rust profile (5 files = max limit)

**Recommendations:**
1. R3 says "Update `packages/core/src/schemas/index.ts` re-exports" but does not explicitly mention adding `QuerySyncInitMessageSchema` to the `MessageSchema` discriminated union. Without this, `QUERY_SYNC_INIT` messages won't parse via the unified `MessageSchema`. Executor should add it to the union alongside existing Query*MessageSchema entries.

**Comment:** Well-structured schema-only spec with clear scope boundaries, correct type choices (`u32` for hashes), and comprehensive test coverage. All 5 delta files verified to exist. Single recommendation is minor — the intent is clear from context.

### Response v1 (2026-03-24)
**Applied:** all

**Changes:**
1. [✓] Added explicit instructions in R3 to import `QuerySyncInitMessageSchema` into `index.ts`, add it to the `MessageSchema` discriminated union, and re-export all new types/schemas

### Audit v2 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Dimensions:**
- Clarity: All 9 requirements specify exact file paths, field names, types, and serde annotations
- Completeness: All 5 delta files verified to exist, all exhaustive match blocks accounted for (ctx(), set_connection_id(), test match)
- Testability: 7 named test cases with specific assertions; all acceptance criteria are verifiable
- Scope: Schema-only boundary clearly enforced by constraints section
- Feasibility: Straightforward additive changes to existing patterns
- Architecture fit: Follows existing Message/Operation/classify patterns exactly
- Non-duplication: Extends existing schemas rather than creating parallel ones
- Cognitive load: Minimal -- mirrors established ShapeSyncInit pattern
- Strategic fit: Step 1/4 of Query/Shape merge aligns with project direction
- Project compliance: u32 for hashes (matches codebase convention, documented deviation from PROJECT.md u64 guidance), 5 files at language profile limit

**Delta validation:** 5/5 entries valid

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs
- [x] `Default` derived on payload structs with 2+ optional fields (QueryRespPayload already has Default)
- [x] Enums used for known value sets (N/A)
- [x] Wire compatibility (`to_vec_named()`)
- [x] `#[serde(rename_all = "camelCase")]` on every struct
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>`

**Strategic fit:** Aligned with project goals (Query/Shape merge step 1/4)
**Project compliance:** Honors PROJECT.md decisions
**Language profile:** Compliant with Rust profile (5 files = max limit)

**Recommendations:**
1. R1 refers to "QuerySubPayload schema object" but the TS schema has no named `QuerySubPayload` -- the payload is an inline `z.object({...})` inside `QuerySubMessageSchema`. The code snippet makes intent clear, but executor should note the field goes into the inline payload object at line 8-12 of `query-schemas.ts`.
2. R3 instructs to "Add re-exports for QuerySyncInitPayload, QuerySyncInitMessage, QuerySyncInitPayloadSchema, QuerySyncInitMessageSchema" in `index.ts`, but `export * from './query-schemas'` at line 10 already re-exports all named exports. The import into `MessageSchema` union is the only required change in `index.ts`. The redundant re-export instruction is harmless but unnecessary.

**Comment:** v1 recommendation successfully addressed. Spec is clear, complete, and ready for implementation. Two minor wording observations documented as recommendations -- neither blocks execution.

### Response v2 (2026-03-24)
**Applied:** all

**Changes:**
1. [✓] R1: Clarified that the payload is an inline `z.object({...})` inside `QuerySubMessageSchema`, not a separately named schema object
2. [✓] R3: Replaced redundant re-export instruction with note that `export * from './query-schemas'` already handles re-exports; only the `MessageSchema` union import is needed

### Audit v3 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Dimensions:**
- Clarity: All 9 requirements specify exact file paths, field names, types, and serde annotations. No vague terms.
- Completeness: All 5 delta files verified to exist. Exhaustive match blocks in `ctx()` and `set_connection_id()` identified. Query domain service uses wildcard catch-all (`_ => Err(OperationError::WrongService)`), so no change needed there.
- Testability: 7 named Rust test cases with specific assertions; all 7 acceptance criteria are measurable and verifiable.
- Scope: Schema-only boundary clearly enforced by constraints section. No scope creep.
- Feasibility: Straightforward additive changes following established patterns.
- Architecture fit: Follows existing Message/Operation/classify patterns exactly. New variant mirrors ShapeSyncInit pattern.
- Non-duplication: Extends existing query schemas rather than creating parallel types.
- Cognitive load: Minimal -- mirrors established ShapeSyncInit pattern throughout.
- Strategic fit: Step 1/4 of Query/Shape merge aligns with project direction (TODO-181 through TODO-184).
- Project compliance: `u32` for hashes matches codebase convention (documented deviation from PROJECT.md `u64` guidance). 5 files at language profile limit.

**Delta validation:** 5/5 entries valid

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields -- `u32` used for hash values
- [x] No `r#type: String` on message structs -- enum owns the tag
- [x] `Default` derived on payload structs with 2+ optional fields -- QueryRespPayload already has Default; new QuerySyncInitPayload has 0 optional fields (correct to omit Default)
- [x] Enums used for known value sets -- N/A (no enum fields added)
- [x] Wire compatibility -- existing codebase uses `to_vec_named()`
- [x] `#[serde(rename_all = "camelCase")]` on every struct -- present in R6
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` -- present in R4, R5

**Strategic fit:** Aligned with project goals
**Project compliance:** Honors PROJECT.md decisions
**Language profile:** Compliant with Rust profile (5 files = max limit)

**Comment:** Both prior recommendations have been addressed. Spec is clear, complete, and ready for implementation. All file paths, types, serde annotations, and test expectations verified against actual source code.

---

## Execution Summary

**Executed:** 2026-03-24
**Commits:** 3

### Files Created
None.

### Files Modified
- `packages/core/src/schemas/query-schemas.ts` — Added `fields` to inline QuerySubMessageSchema payload, `merkleRootHash` to QueryRespPayloadSchema, and new QuerySyncInitPayloadSchema/QuerySyncInitMessageSchema with type exports
- `packages/core/src/schemas/index.ts` — Added QuerySyncInitMessageSchema to import and to MessageSchema discriminated union
- `packages/core-rust/src/messages/query.rs` — Added `fields: Option<Vec<String>>` to QuerySubPayload, `merkle_root_hash: Option<u32>` to QueryRespPayload, new QuerySyncInitPayload/QuerySyncInitMessage structs, and 6 new tests; fixed 5 existing test struct literals for new fields
- `packages/core-rust/src/messages/mod.rs` — Added QuerySyncInit variant to Message enum, added QuerySyncInitMessage/QuerySyncInitPayload re-exports
- `packages/server-rust/src/service/operation.rs` — Added QuerySyncInit variant to Operation enum with match arms in ctx() and set_connection_id(); updated exhaustive test match
- `packages/server-rust/src/service/classify.rs` — Added Message::QuerySyncInit arm routing to service_names::QUERY; added classify_query_sync_init_routes_to_query test; fixed existing classify_query_sub_routes_to_query test struct literal
- `packages/server-rust/src/service/domain/query.rs` — Fixed 4 existing struct literals (3x QuerySubPayload, 1x QueryRespPayload) to include new fields

### Files Deleted
None.

### Acceptance Criteria Status
- [x] QuerySubPayload in both TS and Rust accepts optional `fields` — existing messages without `fields` deserialize unchanged
- [x] QueryRespPayload in both TS and Rust accepts optional `merkleRootHash` / `merkle_root_hash: Option<u32>` — existing messages without it deserialize unchanged
- [x] QUERY_SYNC_INIT message type exists in both TS Zod schema and Rust Message enum with payload `{ queryId, rootHash }`
- [x] QUERY_SYNC_INIT routes to the query service in classify.rs
- [x] All new optional fields use `skip_serializing_if = "Option::is_none"` in Rust and `.optional()` in Zod
- [x] All existing tests pass without modification (backward compatible additions) — 483 core + 610 server tests pass
- [x] New round-trip and camelCase tests pass for all added fields and messages

### Deviations
1. [Rule 1 - Bug] Fixed 5 existing Rust test struct literals in query.rs (classify_query_sub_routes_to_query, query_sub_message_roundtrip, query_sub_camel_case_field_names, query_resp_message_full_roundtrip, query_resp_message_minimal_roundtrip, query_resp_optional_fields_omitted_when_none, query_resp_payload_default_constructs) — adding new fields to structs that were exhaustively specified in tests would cause compilation errors
2. [Rule 1 - Bug] Fixed 4 struct literals in domain/query.rs (3x QuerySubPayload in tests, 1x QueryRespPayload in handler) — same compilation correctness requirement
3. [Rule 1 - Bug] Fixed existing classify_query_sub_routes_to_query test in classify.rs to include `fields: None`

### Notes
- operation.rs counts 6 modified files total (not 5 delta files) because domain/query.rs needed struct-literal fixes — this is a compile-time correctness fix, not a behavioral change
- 610 server tests (+1 from new classify_query_sync_init_routes_to_query test vs 609 baseline)
- 483 core tests (all existing + 6 new query tests)

---

## Review History

### Review v1 (2026-03-24 12:42)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] R1: `fields: z.array(z.string()).optional()` present in inline payload of `QuerySubMessageSchema` in `query-schemas.ts` — correct placement and syntax
- [✓] R2: `merkleRootHash: z.number().int().optional()` present in `QueryRespPayloadSchema` — matches spec exactly
- [✓] R3: `QuerySyncInitPayloadSchema` and `QuerySyncInitMessageSchema` defined with correct field types; types `QuerySyncInitPayload` and `QuerySyncInitMessage` exported; schema imported and placed in `MessageSchema` discriminated union under Query section
- [✓] R4: `fields: Option<Vec<String>>` with correct `#[serde(skip_serializing_if = "Option::is_none", default)]` annotation in `QuerySubPayload`
- [✓] R5: `merkle_root_hash: Option<u32>` with correct serde annotation in `QueryRespPayload`
- [✓] R6: `QuerySyncInitPayload` and `QuerySyncInitMessage` structs present with `#[serde(rename_all = "camelCase")]`, correct field names (`query_id`, `root_hash`), correct type (`u32`) — matches `ShapeSyncInitPayload` pattern
- [✓] R7: `#[serde(rename = "QUERY_SYNC_INIT")] QuerySyncInit(QuerySyncInitMessage)` variant in `Message` enum after `QueryResp`; `QuerySyncInitMessage` and `QuerySyncInitPayload` in re-exports from `query` module
- [✓] R8: `QuerySyncInit` variant in `Operation` enum; present in both `ctx()` and `set_connection_id()` exhaustive match arms; `classify.rs` routes `Message::QuerySyncInit` to `service_names::QUERY`
- [✓] R9: All 7 required tests present and correct — `query_sub_with_fields_roundtrip`, `query_sub_fields_omitted_when_none`, `query_resp_with_merkle_root_hash_roundtrip`, `query_resp_merkle_root_hash_omitted_when_none`, `query_sync_init_roundtrip`, `query_sync_init_camel_case`, `classify_query_sync_init_routes_to_query`
- [✓] SHAPE_* messages not removed — `shape-schemas.ts` still exists, `ShapeSubscribeMessageSchema` etc. still in `MessageSchema` union
- [✓] No behavioral logic added — `QuerySyncInit` is routed but no handler implementation exists in `domain/query.rs`
- [✓] All struct literal fixes in `domain/query.rs` and test files are correct compile-time fixes, not behavioral changes
- [✓] Build check: TS `@topgunbuild/core` builds clean (CJS + ESM + DTS)
- [✓] Rust core tests: 483 passed, 0 failed
- [✓] Rust server tests: 610 passed, 0 failed
- [✓] Clippy: `cargo clippy -- -D warnings` exits 0 (no warnings)
- [✓] Rust auditor checklist: all items pass — `u32` for hashes, no `r#type: String`, `Default` on `QueryRespPayload` (2+ optional fields), `rename_all = "camelCase"` on all new structs, `skip_serializing_if` on all `Option<T>` fields
- [✓] Ordering note: spec says `QuerySyncInitMessageSchema` after `QueryUpdateMessageSchema`; actual placement is before it (line 157 vs 158). This is functionally irrelevant for a discriminated union keyed by `type` literal — order does not affect parsing correctness

**Summary:** All 9 requirements implemented completely and correctly. All 7 acceptance criteria verified by code inspection and passing test runs. No behavioral changes introduced. Clippy clean. The single ordering deviation (QuerySyncInitMessageSchema placed before rather than after QueryUpdateMessageSchema in the union) has no functional effect.

---

## Completion

**Completed:** 2026-03-24
**Total Commits:** 3
**Review Cycles:** 1

### Outcome

Extended QUERY_SUB/QUERY_RESP wire messages with Shape capabilities (fields projection, merkleRootHash) and added QUERY_SYNC_INIT message type — step 1/4 of the Query/Shape unification (TODO-181–184).

### Key Files

- `packages/core/src/schemas/query-schemas.ts` — TS query message schemas with new fields and QUERY_SYNC_INIT
- `packages/core-rust/src/messages/query.rs` — Rust query message structs with new fields, QuerySyncInit types, and 6 new tests
- `packages/server-rust/src/service/classify.rs` — QUERY_SYNC_INIT routing to query service

### Changes Applied

**Modified:**
- `packages/core/src/schemas/query-schemas.ts` — Added optional `fields` to QuerySub payload, optional `merkleRootHash` to QueryResp payload, new QuerySyncInit schema+types
- `packages/core/src/schemas/index.ts` — Added QuerySyncInitMessageSchema to MessageSchema discriminated union
- `packages/core-rust/src/messages/query.rs` — Added `fields: Option<Vec<String>>` to QuerySubPayload, `merkle_root_hash: Option<u32>` to QueryRespPayload, new QuerySyncInitPayload/QuerySyncInitMessage structs, 6 new tests
- `packages/core-rust/src/messages/mod.rs` — Added QuerySyncInit variant to Message enum, re-exports
- `packages/server-rust/src/service/operation.rs` — Added QuerySyncInit variant to Operation enum
- `packages/server-rust/src/service/classify.rs` — Added QuerySyncInit classify arm routing to query service

### Deviations from Delta

- `packages/server-rust/src/service/domain/query.rs` — Not in delta but required struct literal fixes (adding `fields: None`, `merkle_root_hash: None`) to 4 existing struct literals for compilation correctness
- `packages/core/src/schemas/index.ts` — Not in delta but required adding QuerySyncInitMessageSchema to MessageSchema union import

### Patterns Established

None — followed existing patterns.

### Spec Deviations

None — implemented as specified. All deviations were compile-correctness fixes to existing struct literals.
