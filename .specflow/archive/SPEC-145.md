---
id: SPEC-145
type: refactor
status: done
priority: P1
complexity: large
delta: true
created: 2026-03-24
source: TODO-184
---

# Remove Deprecated Shape Code (Query+Shape Merge Step 4/4)

## Context

Steps 1-3 of the Query+Shape merge are complete:
- SPEC-142 (TODO-181): Extended wire protocol — QUERY_SUB gained `fields`, QUERY_RESP gained `merkleRootHash`, added QUERY_SYNC_INIT message
- SPEC-143 (TODO-182): Merged Shape capabilities into QueryService — field projection, per-query Merkle trees, QUERY_SYNC_INIT handler
- SPEC-144 (TODO-183): Merged ShapeHandle/ShapeManager into QueryHandle/QueryManager on the client

All Shape functionality now lives under the Query path. The Shape-specific code is dead — no wire messages reference it, no client calls it. This spec deletes it entirely. No backwards compatibility is needed (no active users).

## Language Profile Exemption

This spec touches 15 Rust files (4 deleted + 11 modified across core-rust and server-rust), exceeding the Language Profile limit of 5 Rust files per spec. The exemption applies because:
- All Rust changes are deletions or 1–5 line removals of `mod`, `pub use`, `pub mod`, and enum variant declarations
- No new structs, traits, or implementations are introduced
- Borrow checker cascade risk is minimal for pure deletion work
- Splitting into 3 Rust-scoped specs would create artificial ordering constraints with no implementation benefit

This exemption is specific to deletion-only specs. Feature specs adding or modifying Rust code remain subject to the 5-file limit.

## Goal Analysis

**Goal:** Eliminate all dead Shape code so the codebase has exactly one path for filtered subscriptions (Queries).

**Observable Truths:**
1. No Shape-related message types exist in wire protocol (TS schemas or Rust enum)
2. No ShapeService, ShapeRegistry, or ShapeMerkleSyncManager exist in server code
3. No ShapeHandle, ShapeManager, or subscribeShape exist in client code
4. No shape-schemas.ts or shape.rs message module exists in core packages
5. No shape integration tests or shape-specific unit tests exist
6. No shapes.mdx guide page exists; live-queries.mdx documents fields/limit/Merkle reconnect
7. Server compiles, all tests pass, client builds cleanly

## Delta

### REMOVED
- `packages/core-rust/src/messages/shape.rs` — Shape message structs (ShapeSubPayload, ShapeRespPayload, etc.)
- `packages/server-rust/src/service/domain/shape.rs` — ShapeService and ShapeRegistry
- `packages/server-rust/src/service/domain/shape_evaluator.rs` — Shape field evaluator (project() must be inlined into query.rs as project_fields() before this file is deleted; SyncShape type must be verified unused by query.rs before deletion)
- `packages/server-rust/src/storage/shape_merkle.rs` — ShapeMerkleSyncManager
- `packages/client/src/ShapeHandle.ts` — Deprecated ShapeHandle class
- `packages/client/src/sync/ShapeManager.ts` — Deprecated ShapeManager class
- `packages/core/src/schemas/shape-schemas.ts` — Shape Zod schemas
- `tests/integration-rust/shape.test.ts` — Shape integration tests (replaced by queries.test.ts)
- `apps/docs-astro/src/content/docs/guides/shapes.mdx` — Shapes guide page

### MODIFIED
- `packages/core-rust/src/messages/mod.rs` — Remove `mod shape`, Shape variants from Message enum, re-exports
- `packages/server-rust/src/service/operation.rs` — Remove Operation::ShapeSubscribe/ShapeUnsubscribe/ShapeSyncInit variants and routing arms
- `packages/server-rust/src/service/classify.rs` — Remove SHAPE_SUBSCRIBE/SHAPE_UNSUBSCRIBE/SHAPE_SYNC_INIT classification branches
- `packages/server-rust/src/service/domain/mod.rs` — Remove `pub mod shape`, `pub mod shape_evaluator`, `pub use shape::ShapeService`
- `packages/server-rust/src/storage/mod.rs` — Remove `pub mod shape_merkle`
- `packages/server-rust/src/lib.rs` — Remove ShapeService/ShapeRegistry/ShapeMerkleSyncManager imports, construction, and registration from server assembly
- `packages/server-rust/src/bin/test_server.rs` — Remove shape-related setup if present
- `packages/server-rust/src/service/domain/crdt.rs` — Remove shape_registry parameter and any Shape-related broadcast logic
- `packages/server-rust/src/service/domain/sync.rs` — Remove shape_merkle references
- `packages/server-rust/src/service/domain/query.rs` — Inline project() from shape_evaluator as project_fields(); remove shape_registry constructor parameter; verify SyncShape is not used and remove its import if so
- `packages/server-rust/src/network/module.rs` — Remove shape-related handler registrations if present
- `packages/client/src/SyncEngine.ts` — Remove subscribeShape() method and shape-related imports
- `packages/client/src/sync/ClientMessageHandlers.ts` — Remove SHAPE_RESP/SHAPE_UPDATE message handlers
- `packages/client/src/index.ts` — Remove ShapeHandle/ShapeUpdate re-exports
- `packages/client/src/sync/index.ts` — Remove ShapeManager re-export
- `packages/core/src/schemas/index.ts` — Remove shape-schemas re-exports
- `apps/docs-astro/src/content/docs/guides/live-queries.mdx` — Add documentation for `fields` (field projection), `limit`, and Merkle delta reconnect features

## Requirements

### R1: Delete Rust Shape message types
Delete `packages/core-rust/src/messages/shape.rs`. Remove `mod shape` declaration and all Shape-related re-exports from `packages/core-rust/src/messages/mod.rs`. Remove SHAPE_SUBSCRIBE, SHAPE_UNSUBSCRIBE, SHAPE_RESP, SHAPE_UPDATE, SHAPE_SYNC_INIT variants from the Message enum.

Note: `SyncShape` is NOT defined in `messages/shape.rs`. It is defined in `core-rust/src/schema.rs` and only imported by `shape.rs`. `SyncShape` belongs to the schema module and is intentionally retained — do not remove it.

If for any reason `SyncShape` appears to be re-exported through `messages/mod.rs` into non-shape files, verify its origin in `schema.rs` before taking any action. The conditional logic below applies only to any other types that may reside in `shape.rs`: if any such type is NOT referenced anywhere in `query.rs` or other non-shape files, remove it. If `query.rs` references it, retain the type by moving its definition into `query.rs` or a shared types module before deleting `shape.rs`.

### R2: Delete Rust Shape server code

**Step 2a — Inline project() into query.rs before deletion:**
Inspect `packages/server-rust/src/service/domain/shape_evaluator.rs`. Copy the `project()` function body into `packages/server-rust/src/service/domain/query.rs` as a private function named `project_fields()`. Update all call sites within `query.rs` (including test references at lines 1726, 1763, 1797) to call `project_fields()` instead of `super::shape_evaluator::project()`. Confirm the build succeeds with the inlined function before proceeding.

**Step 2b — Verify SyncShape usage:**
Check whether `query.rs` imports or uses `SyncShape` from `shape_evaluator` (beyond the `project()` function). If it does, handle per R1. If it does not, proceed.

**Step 2c — Delete files:**
Delete these files:
- `packages/server-rust/src/service/domain/shape.rs`
- `packages/server-rust/src/service/domain/shape_evaluator.rs`
- `packages/server-rust/src/storage/shape_merkle.rs`

Remove from `packages/server-rust/src/service/domain/mod.rs`: `pub mod shape`, `pub mod shape_evaluator`, `pub use shape::ShapeService`.
Remove from `packages/server-rust/src/storage/mod.rs`: `pub mod shape_merkle`.

### R3: Remove Shape routing from Operation pipeline
In `packages/server-rust/src/service/operation.rs`: delete Operation::ShapeSubscribe, Operation::ShapeUnsubscribe, Operation::ShapeSyncInit enum variants and all match arms referencing them.
In `packages/server-rust/src/service/classify.rs`: delete the classification branches that produce these three Operation variants.
Remove `service_names::SHAPE` constant if defined.

### R4: Remove Shape wiring from server assembly
In `packages/server-rust/src/lib.rs`: remove ShapeService, ShapeRegistry, ShapeMerkleSyncManager imports. Remove their construction (`ShapeRegistry::new()`, `ShapeMerkleSyncManager::new()`). Remove ShapeService registration on the router. Remove `shape_registry` and `shape_merkle_manager` parameters passed to CrdtService, SyncService, and QueryService constructors.

Update CrdtService, SyncService, and QueryService constructors to no longer accept optional shape_registry/shape_merkle_manager parameters. Remove the corresponding fields from those service structs.

When removing the `shape_registry` field from `CrdtService`, also remove the 4 shape-specific test functions that depend on it: `shape_broadcast_enter_on_newly_matching_write`, `shape_broadcast_update_on_matching_to_matching_write`, `shape_broadcast_leave_on_matching_to_non_matching_write`, and `shape_broadcast_skips_non_matching_to_non_matching` (approximately lines 2004–2199 of `crdt.rs`), along with their helpers `make_shape_service` and `drain_shape_updates`. These tests exercise the removed ShapeRegistry functionality and will fail to compile once the field is gone.

Update `packages/server-rust/src/bin/test_server.rs` similarly.

Note: The `project()` function inlining (as `project_fields()`) is handled in R2 step 2a and must be complete before R4 is executed.

### R5: Delete TS client Shape code
Delete:
- `packages/client/src/ShapeHandle.ts`
- `packages/client/src/sync/ShapeManager.ts`

Remove `subscribeShape()` method from `packages/client/src/SyncEngine.ts`.
Remove SHAPE_RESP and SHAPE_UPDATE handlers from `packages/client/src/sync/ClientMessageHandlers.ts`.
Remove ShapeHandle/ShapeUpdate exports from `packages/client/src/index.ts`.
Remove ShapeManager export from `packages/client/src/sync/index.ts`.

### R6: Delete TS core Shape schemas
Delete `packages/core/src/schemas/shape-schemas.ts`.
Remove shape schema re-exports from `packages/core/src/schemas/index.ts`.

### R7: Delete Shape tests
Delete `tests/integration-rust/shape.test.ts`.
Delete any shape-specific unit tests in `packages/server-rust/src/service/domain/shape.rs` (deleted with the file).
Delete the 4 shape-specific test functions and their helpers from `packages/server-rust/src/service/domain/crdt.rs` as described in R4.

### R8: Update documentation
Delete `apps/docs-astro/src/content/docs/guides/shapes.mdx`.
Update `apps/docs-astro/src/content/docs/guides/live-queries.mdx` to document:
- `fields` option for field projection (previously Shape-only)
- `limit` option for result capping
- Merkle delta reconnect behavior (automatic efficient resync)

Remove any sidebar/nav references to the shapes guide page. Fix pagination links if shapes.mdx was adjacent to other pages.

### R9: Archive TODO-070
Mark TODO-070 (Shapes concept) as superseded in TODO.md — Shapes have been merged into Queries via TODO-181/182/183/184.

## Acceptance Criteria

1. `cargo build -p topgun-server` succeeds with zero warnings related to shape
2. `cargo test --release -p topgun-server` passes — no shape test failures (they are deleted)
3. `pnpm --filter @topgunbuild/core build` succeeds with no shape-schemas references
4. `pnpm --filter @topgunbuild/client build` succeeds with no ShapeHandle/ShapeManager references
5. `pnpm --filter @topgunbuild/client test` passes
6. `pnpm test:integration-rust` passes — no shape.test.ts (deleted)
7. `grep -r "ShapeService\|ShapeRegistry\|ShapeMerkleSyncManager\|ShapeHandle\|ShapeManager\|SHAPE_SUBSCRIBE\|SHAPE_UNSUBSCRIBE\|SHAPE_RESP\|SHAPE_UPDATE\|SHAPE_SYNC_INIT\|shape_evaluator\|shape_merkle\|shape-schemas" packages/ tests/` returns zero matches
8. `live-queries.mdx` contains sections on field projection, limit, and Merkle reconnect
9. `shapes.mdx` does not exist
10. `grep -r "project_fields" packages/server-rust/src/service/domain/query.rs` returns at least one match (confirming the inlined function exists)

## Validation Checklist

1. Run `cargo build -p topgun-server 2>&1 | grep -i shape` — no output
2. Run `pnpm build 2>&1` — all packages build cleanly
3. Run `pnpm test:integration-rust` — all pass, no shape tests
4. Run `grep -ri "shape" packages/server-rust/src/ packages/core-rust/src/ packages/client/src/ packages/core/src/schemas/ --include="*.rs" --include="*.ts" | grep -v "SyncShape"` — zero matches. Note: `SyncShape` matches are expected and acceptable; the type is defined in `core-rust/src/schema.rs` as part of the schema system and is intentionally retained. The `| grep -v "SyncShape"` filter excludes these from the check. The inlined function is named `project_fields`, not `project_shape`, so it will not produce false positives.
5. Open `apps/docs-astro/src/content/docs/guides/live-queries.mdx` — contains fields, limit, Merkle reconnect documentation
6. Run `grep "project_fields" packages/server-rust/src/service/domain/query.rs` — confirms inlined function present

## Constraints

- Do NOT remove `shape_evaluator` logic if any of it is used by the query module — verify before deleting. `project()` is actively imported by `query.rs`; inline it as `project_fields()` into `query.rs` before deleting `shape_evaluator.rs`.
- Do NOT modify QueryHandle, QueryManager, or QueryService functionality — only remove Shape-specific parameters they no longer use.
- Do NOT change wire protocol message IDs for existing Query messages.
- Do NOT remove any query-related tests — only shape-specific tests.

## Assumptions

- `project()` in `shape_evaluator.rs` is actively imported by `query.rs` (lines 587, 1726, 1763, 1797). It must be inlined as `project_fields()` into `query.rs` before `shape_evaluator.rs` is deleted.
- `SyncShape` is defined in `core-rust/src/schema.rs` (not in `messages/shape.rs`). `messages/shape.rs` only imports `SyncShape` from `crate::schema`. `SyncShape` is NOT part of the dead Shape feature code and must be retained. It is not used by `query.rs` (0 grep matches confirmed in Audit v2).
- The shapes.mdx page has no inbound links from other guide pages (only sidebar/nav). If other pages link to it, those links must be updated to point to live-queries.mdx.
- TODO-070 exists in TODO.md and can be marked as superseded with a one-line note.
- The `service_names::SHAPE` constant exists alongside other service name constants and can be removed.

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | R1: Delete Rust shape message types from core-rust | — | ~10% |
| G2 | 1 | R6: Delete TS core shape schemas | — | ~5% |
| G3 | 2 | R2+R3: Inline project() as project_fields() into query.rs, verify SyncShape, delete Rust shape server code + remove Operation routing | G1 | ~20% |
| G4 | 2 | R5: Delete TS client shape code | G2 | ~15% |
| G5 | 3 | R4: Remove shape wiring from server assembly (lib.rs, test_server.rs, service constructors) | G3 | ~15% |
| G6 | 3 | R7: Delete shape tests | G4 | ~5% |
| G7 | 4 | R8+R9: Update docs, archive TODO-070 | G5, G6 | ~20% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |
| 3 | G5, G6 | Yes | 2 |
| 4 | G7 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-24)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (26 files: 9 removed + 17 modified)

**Critical:**
1. **shape_evaluator::project() inlining not assigned to any task group.** Verified that `query.rs` actively imports `super::shape_evaluator::project()` (line 587) and its tests reference `shape_evaluator` (lines 1726, 1763, 1797). The spec's constraint section correctly says "inline the needed functions into query.rs first," but no requirement or task group includes this inlining step. G3 (Wave 2) deletes `shape_evaluator.rs`, but G5 (Wave 3) only removes "shape_registry parameter" from query.rs -- the `project()` function inlining is not covered. R2 must explicitly state: "Before deleting shape_evaluator.rs, inline the `project()` function (lines 32-47) into query.rs and update query.rs imports accordingly." This inlining must happen in the same task group that deletes shape_evaluator.rs (G3), or in an earlier group.
2. **Delta description is factually incorrect.** Line 41 says shape_evaluator.rs has "functionality already in query module" -- this is false. The `project()` function is NOT duplicated; query.rs imports it from shape_evaluator. The Delta REMOVED entry must be corrected to: "Shape field evaluator (project() must be inlined into query.rs before deletion)."
3. **Language Profile violation: 15 Rust files exceeds 5-file limit.** PROJECT.md Language Profile specifies "Max files per spec: 5" for Rust packages. This spec touches 15 Rust files (4 removed + 11 modified across core-rust and server-rust). Total across all languages: 26 files. The profile rationale is "limit borrow checker cascade risk," which is lower for deletion work, but the rule is explicit. Resolution options: (a) split into 3 Rust-scoped specs (core-rust messages, server-rust domain/routing, server-rust assembly/wiring) plus 1 TS spec, or (b) add an explicit Language Profile exemption note for deletion-only specs to PROJECT.md. Given that most Rust file modifications are 1-5 line removals (deleting `mod` declarations, removing `pub use` lines), option (b) is more pragmatic -- but the spec must acknowledge the deviation.

**Recommendations:**
4. [Compliance] R4 description says "Remove shape_registry parameter from QueryService::new() if passed" but should also mention inlining `project()` -- currently R4 only covers constructor parameters, not the function dependency. Consider merging the inlining step into R2 explicitly with a sub-step.
5. G5 estimated at ~25% context seems high for removal-only changes to lib.rs, test_server.rs, and service constructors. Since these are deletion operations (removing parameters and fields), ~15% is more realistic.
6. Validation Checklist item 4 greps for "shape" broadly but the inlined `project()` function itself should be renamed (e.g., `project_fields`) to avoid any "shape" references in its vicinity. The spec should state this explicitly.
7. The `SyncShape` type from core-rust is referenced by shape_evaluator's `matches()` and `apply_shape()` functions. Verify that `SyncShape` itself can be removed or is already unused by query.rs. If query.rs uses `SyncShape`, that type must be retained or renamed.

**Delta validation:** 26/26 entries valid (all REMOVED files exist, all MODIFIED files exist)

**Goal-Backward Validation:**
| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (no Shape messages) has artifacts | Pass | R1, R6 cover wire protocol cleanup |
| Truth 2 (no Shape server code) has artifacts | Pass | R2, R3, R4 cover server cleanup |
| Truth 3 (no Shape client code) has artifacts | Pass | R5 covers client cleanup |
| Truth 7 (compiles, tests pass) has artifacts | Pass | AC1-6 verify builds and tests |
| project() inlining wired correctly | FAIL | No task group handles inlining before deletion |

**Strategic fit:** Aligned with project goals -- this is the final step of a 4-part merge that eliminates API duplication.

**Project compliance:** Compliant with all PROJECT.md decisions except Language Profile file count (see Critical #3).

### Response v1 (2026-03-24)
**Applied:** All 3 critical issues and all 4 recommendations.

**Changes:**
1. [✓] project() inlining not in any task group — Added R2 steps 2a/2b/2c making inlining of project() as project_fields() an explicit prerequisite sub-step within G3 before shape_evaluator.rs is deleted. Updated G3 description to reference inlining. Added R4 note confirming inlining is handled in R2. Added AC10 to verify project_fields() exists. Added Validation step 6.
2. [✓] Delta description factually incorrect — Corrected shape_evaluator.rs REMOVED entry from "functionality already in query module" to "project() must be inlined into query.rs as project_fields() before this file is deleted; SyncShape type must be verified unused by query.rs before deletion."
3. [✓] Language Profile violation — Added explicit Language Profile Exemption section after Context, documenting the rationale for exceeding the 5-file limit for deletion-only specs.
4. [✓] R4 should mention project() inlining — Added explicit note to R4: "The project() function inlining (as project_fields()) is handled in R2 step 2a and must be complete before R4 is executed."
5. [✓] G5 context estimate too high — Reduced G5 estimated context from ~25% to ~15%.
6. [✓] Inlined function should be renamed to avoid "shape" grep hits — All occurrences of inlining description now specify project_fields() as the target name. Added explicit note to Validation Checklist item 4 that project_fields() will not produce false positives in the shape grep. Updated Constraints section accordingly.
7. [✓] SyncShape type verification — Added Step 2b to R2 requiring explicit verification that query.rs does not use SyncShape before deletion. Updated R1 to handle SyncShape retention or removal based on verification result. Updated Assumptions to document the expected finding (SyncShape not used by query.rs) and the fallback if the assumption is wrong. Updated Delta REMOVED entry for shape_evaluator.rs to reference SyncShape verification.

### Audit v2 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~90% total (26 files: 9 removed + 17 modified)

All three v1 critical issues have been resolved:
- project() inlining is now explicitly covered in R2 steps 2a/2b/2c within G3
- Delta description for shape_evaluator.rs is accurate
- Language Profile exemption is documented with clear rationale

**Verified assumptions:**
- `project()` is imported by query.rs at lines 587, 1726, 1763, 1797 -- confirmed
- `SyncShape` is NOT used by query.rs -- confirmed (0 grep matches)
- `SyncShape` is defined in `core-rust/src/schema.rs` (not in `messages/shape.rs`) -- confirmed; it is NOT part of shape dead code
- `service_names::SHAPE` exists in operation.rs (line 36) -- confirmed
- `network/module.rs` has `shape_registry: None` (line 271) -- confirmed, covered by Delta
- TODO-070 exists in TODO.md -- confirmed
- All 9 REMOVED files exist -- confirmed
- All 17 MODIFIED files exist -- confirmed

**Delta validation:** 26/26 entries valid

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~90% | <=50% | -- |
| Largest task group | ~20% (G3) | <=30% | ok |
| Worker overhead | ~20% (4 waves x ~5%) | <=10% | -- |

Note: The high total context estimate reflects 26 file touches, but since nearly all changes are deletions or 1-5 line removals, actual cognitive complexity per group is low. The Language Profile Exemption section documents why this is acceptable for a deletion-only spec.

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | -- estimate lands here nominally |

The nominal estimate is high because file count is large, but deletion work has a much lower complexity multiplier than creation or modification. Each worker group stays under 20%, which is in the PEAK range individually. With orchestrated parallel execution, quality should remain GOOD per worker.

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (no Shape messages) has artifacts | Pass | R1, R6 |
| Truth 2 (no Shape server code) has artifacts | Pass | R2, R3, R4 |
| Truth 3 (no Shape client code) has artifacts | Pass | R5 |
| Truth 4 (no shape modules in core) has artifacts | Pass | R1, R6 |
| Truth 5 (no shape tests) has artifacts | Pass | R7 + file deletions |
| Truth 6 (docs updated) has artifacts | Pass | R8 |
| Truth 7 (compiles, tests pass) has artifacts | Pass | AC1-6 |
| project() inlining wired correctly | Pass | R2 step 2a in G3 |

**Strategic fit:** Aligned with project goals -- final cleanup of a 4-part API merge.

**Project compliance:** Compliant. Language Profile exemption documented and justified for deletion-only work.

**Recommendations:**
1. **R4/R7 should explicitly mention crdt.rs shape tests.** The `crdt.rs` file contains 4 shape-specific test functions (`shape_broadcast_enter_on_newly_matching_write`, `shape_broadcast_update_on_matching_to_matching_write`, `shape_broadcast_leave_on_matching_to_non_matching_write`, `shape_broadcast_skips_non_matching_to_non_matching` at lines 2004-2199+) plus helpers (`make_shape_service`, `drain_shape_updates`). These must be removed when the `shape_registry` field is removed from CrdtService (R4). R7 only mentions shape.rs tests, not crdt.rs shape tests. The compiler will catch this (removing the field breaks the tests), but an explicit note in R4 or R7 would prevent confusion.
2. **Validation Checklist item 4 will produce false positives.** The broad `grep -ri "shape"` will match `SyncShape` in `core-rust/src/schema.rs`, `server-rust/src/traits.rs`, `server-rust/src/service/domain/schema.rs`, and `server-rust/src/service/domain/datafusion_backend.rs`. The `SyncShape` type is part of the schema system (not shape-specific dead code) and is intentionally retained. Item 4 should either: (a) exclude `SyncShape` from the grep pattern, or (b) note that `SyncShape` matches are expected and acceptable since the type belongs to the schema module, not the deprecated Shape feature.
3. **R1 SyncShape guidance is slightly misleading.** R1 says "If SyncShape is defined in shape.rs..." but `SyncShape` is actually defined in `core-rust/src/schema.rs` (verified), not in `messages/shape.rs`. The `messages/shape.rs` file only imports `SyncShape` from `crate::schema`. R1's conditional logic is correct (it will find SyncShape is not in shape.rs and skip it), but the framing could confuse an implementer. A clarifying note that `SyncShape` lives in `schema.rs` and is intentionally retained would be helpful.

**Comment:** Spec is well-structured after v1 revisions. The R2 step-by-step approach (inline first, verify, then delete) correctly handles the main risk. All v1 critical issues are fully addressed. The three recommendations above are non-blocking -- the compiler will enforce correctness for #1, and #2/#3 are documentation clarity improvements.

### Response v2 (2026-03-24)
**Applied:** All 3 recommendations from Audit v2.

**Changes:**
1. [✓] R4/R7 should explicitly mention crdt.rs shape tests — Added explicit paragraph to R4 listing all 4 shape-specific test functions by name (`shape_broadcast_enter_on_newly_matching_write`, `shape_broadcast_update_on_matching_to_matching_write`, `shape_broadcast_leave_on_matching_to_non_matching_write`, `shape_broadcast_skips_non_matching_to_non_matching`) plus helpers `make_shape_service` and `drain_shape_updates` with approximate line range (2004–2199). Updated R7 to reference these crdt.rs shape tests explicitly, pointing back to R4.
2. [✓] Validation Checklist item 4 false positives — Updated item 4 to pipe through `| grep -v "SyncShape"` and added explanatory note that SyncShape matches in `core-rust/src/schema.rs`, `server-rust/src/traits.rs`, `server-rust/src/service/domain/schema.rs`, and `server-rust/src/service/domain/datafusion_backend.rs` are expected and intentionally retained.
3. [✓] R1 SyncShape guidance misleading — Replaced the conditional "if SyncShape is defined in shape.rs" framing with a direct clarifying note: `SyncShape` is defined in `core-rust/src/schema.rs` and only imported by `shape.rs`; it belongs to the schema system and must be retained. Updated Assumptions section to reflect the confirmed finding from Audit v2.

### Audit v3 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~90% total (26 files: 9 removed + 17 modified)

All three Audit v2 recommendations have been properly applied in Response v2:
- R4 now explicitly lists all 4 crdt.rs shape test functions and helpers by name with line ranges
- Validation Checklist item 4 now pipes through `| grep -v "SyncShape"` with explanatory note
- R1 SyncShape guidance replaced with direct clarifying note about `schema.rs` origin

**Delta validation:** 26/26 entries valid (all REMOVED files exist, all MODIFIED files exist)

**Goal-Backward Validation:** All 7 observable truths have artifact coverage. project() inlining is correctly wired in R2 step 2a within G3.

**Strategic fit:** Aligned with project goals -- final cleanup of a 4-part API merge.

**Project compliance:** Compliant. Language Profile exemption documented and justified for deletion-only work.

**Comment:** Spec is thorough and implementation-ready after two rounds of revision. Requirements are precise with explicit file paths, function names, and line references. The R2 step-by-step approach (inline, verify, delete) correctly mitigates the main risk. Acceptance criteria are measurable and grep-verifiable. No remaining issues.

---

## Execution Summary

**Executed:** 2026-03-24
**Mode:** orchestrated
**Commits:** 7

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3, G4 | complete |
| 3 | G5, G6 | complete |
| 4 | G7 | complete |

### Files Deleted
- `packages/core-rust/src/messages/shape.rs` — Shape message structs
- `packages/server-rust/src/service/domain/shape.rs` — ShapeService and ShapeRegistry
- `packages/server-rust/src/service/domain/shape_evaluator.rs` — Shape field evaluator (project() inlined as project_fields() into query.rs)
- `packages/server-rust/src/storage/shape_merkle.rs` — ShapeMerkleSyncManager
- `packages/client/src/ShapeHandle.ts` — Deprecated ShapeHandle class
- `packages/client/src/sync/ShapeManager.ts` — Deprecated ShapeManager class
- `packages/core/src/schemas/shape-schemas.ts` — Shape Zod schemas
- `tests/integration-rust/shape.test.ts` — Shape integration tests
- `apps/docs-astro/src/content/docs/guides/shapes.mdx` — Shapes guide page

### Files Modified
- `packages/core-rust/src/messages/mod.rs` — Removed mod shape, Shape variants from Message enum, re-exports
- `packages/server-rust/src/service/operation.rs` — Removed Shape Operation variants and routing arms
- `packages/server-rust/src/service/classify.rs` — Removed Shape classification branches
- `packages/server-rust/src/service/domain/mod.rs` — Removed pub mod shape, shape_evaluator
- `packages/server-rust/src/storage/mod.rs` — Removed pub mod shape_merkle
- `packages/server-rust/src/lib.rs` — Removed Shape wiring from server assembly
- `packages/server-rust/src/bin/test_server.rs` — Removed shape-related setup
- `packages/server-rust/src/service/domain/crdt.rs` — Removed shape_registry parameter, 4 shape test functions, helpers
- `packages/server-rust/src/service/domain/sync.rs` — Removed shape_merkle references
- `packages/server-rust/src/service/domain/query.rs` — Inlined project_fields(), removed shape references
- `packages/server-rust/src/network/module.rs` — Removed shape_registry: None
- `packages/server-rust/src/storage/query_merkle.rs` — Cleaned stale Shape comments
- `packages/client/src/SyncEngine.ts` — Removed subscribeShape() method
- `packages/client/src/sync/ClientMessageHandlers.ts` — Removed SHAPE_RESP/SHAPE_UPDATE handlers
- `packages/client/src/index.ts` — Removed ShapeHandle/ShapeUpdate re-exports
- `packages/client/src/sync/index.ts` — Removed ShapeManager re-export
- `packages/core/src/schemas/index.ts` — Removed shape schema re-exports
- `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts` — Updated message type count, removed shapeManager mocks
- `apps/docs-astro/src/content/docs/guides/live-queries.mdx` — Added fields, limit, Merkle reconnect docs
- `.specflow/todos/TODO.md` — Marked TODO-070 as superseded

### Acceptance Criteria Status
- [x] AC1: cargo build -p topgun-server succeeds with zero shape warnings
- [x] AC2: cargo test --release -p topgun-server passes (589 tests)
- [x] AC3: pnpm --filter @topgunbuild/core build succeeds
- [x] AC4: pnpm --filter @topgunbuild/client build succeeds
- [x] AC5: pnpm --filter @topgunbuild/client test passes (461 tests)
- [x] AC6: pnpm test:integration-rust passes (57 tests, 7 suites)
- [x] AC7: Zero matches for shape-related symbols in packages/ tests/
- [x] AC8: live-queries.mdx contains fields, limit, Merkle reconnect docs
- [x] AC9: shapes.mdx does not exist
- [x] AC10: project_fields() exists in query.rs

### Deviations
1. [Rule 1 - Bug] Fixed 2 CrdtService::new() test calls in crdt.rs that still passed `None` as 6th arg (removed shape_registry)
2. [Rule 1 - Bug] Updated CLIENT_MESSAGE_TYPES count test from 35 to 33, removed stale shapeManager mocks
3. [Rule 2 - Missing Critical] Cleaned 6 stale Shape references in comments (query.rs, query_merkle.rs) to pass AC7 grep check

### Notes
- SyncShape type in core-rust/src/schema.rs intentionally retained (schema system, not Shape feature)
- Server test count dropped from ~617 to 589 due to removal of 4 shape-specific crdt.rs tests and shape.rs unit tests

---

## Review History

### Review v1 (2026-03-24 20:49)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `cargo build -p topgun-server` succeeds with zero warnings — clean build in 3.09s
- [✓] AC2: `cargo test --release -p topgun-server` passes — 589 tests, 0 failures
- [✓] AC3: `pnpm --filter @topgunbuild/core build` succeeds cleanly
- [✓] AC4: `pnpm --filter @topgunbuild/client build` succeeds cleanly
- [✓] AC5: `pnpm --filter @topgunbuild/client test` passes — 461 tests, 0 failures
- [✓] AC6: `pnpm test:integration-rust` passes — 57 tests, 7 suites, no shape.test.ts
- [✓] AC7: Zero matches for all shape-related symbols (ShapeService, ShapeRegistry, ShapeMerkleSyncManager, ShapeHandle, ShapeManager, SHAPE_SUBSCRIBE, SHAPE_UNSUBSCRIBE, SHAPE_RESP, SHAPE_UPDATE, SHAPE_SYNC_INIT, shape_evaluator, shape_merkle, shape-schemas) in packages/ and tests/
- [✓] AC8: `live-queries.mdx` contains fields projection, limit, and Merkle delta reconnect documentation
- [✓] AC9: `shapes.mdx` does not exist
- [✓] AC10: `project_fields()` exists in `query.rs` with multiple call sites (function inlined correctly)
- [✓] All 9 specified files deleted — confirmed DELETED for every entry
- [✓] All 17 modified files exist — confirmed EXISTS for every entry
- [✓] Operation enum has no Shape variants — `operation.rs` contains only Query, Sync, CRDT, Messaging, Coordination, Search, Persistence, System variants
- [✓] `classify.rs` has no Shape classification branches
- [✓] `service/domain/mod.rs` and `storage/mod.rs` have no shape module declarations
- [✓] `SyncShape` correctly retained in `core-rust/src/schema.rs` — the grep `| grep -v "SyncShape"` filter confirms no false positives in shape removal
- [✓] `TODO-070` marked as superseded in TODO.md
- [✓] `CLIENT_MESSAGE_TYPES` count updated from 35 to 33 in test file
- [✓] `shapes.mdx` has no sidebar/nav references remaining in astro.config.mjs or source docs
- [✓] No shape references in `lib.rs`, `network/module.rs`, `crdt.rs`, `sync.rs`, `query.rs` (beyond `project_fields` which has no "shape" in the name)

**Summary:** All 10 acceptance criteria pass. All 9 files deleted, all modified files contain no residual Shape code. The `project_fields()` inlining is correct. Tests pass across all layers (Rust unit, TS client, integration). The implementation is clean, complete, and fully compliant with the specification.

---

## Completion

**Completed:** 2026-03-24
**Total Commits:** 7
**Review Cycles:** 1

### Outcome

Removed all deprecated Shape code from the codebase, completing the 4-step Query+Shape merge. The codebase now has exactly one path for filtered subscriptions (Queries) with field projection, limit, and Merkle delta reconnect capabilities.

### Key Files

- `packages/server-rust/src/service/domain/query.rs` — Now contains inlined `project_fields()` function, the sole path for field projection
- `apps/docs-astro/src/content/docs/guides/live-queries.mdx` — Updated documentation covering fields, limit, and Merkle reconnect

### Changes Applied

**Modified:**
- `packages/core-rust/src/messages/mod.rs` — Removed mod shape, Shape variants from Message enum, re-exports
- `packages/server-rust/src/service/operation.rs` — Removed Shape Operation variants and routing arms
- `packages/server-rust/src/service/classify.rs` — Removed Shape classification branches
- `packages/server-rust/src/service/domain/mod.rs` — Removed pub mod shape, shape_evaluator
- `packages/server-rust/src/storage/mod.rs` — Removed pub mod shape_merkle
- `packages/server-rust/src/lib.rs` — Removed Shape wiring from server assembly
- `packages/server-rust/src/bin/test_server.rs` — Removed shape-related setup
- `packages/server-rust/src/service/domain/crdt.rs` — Removed shape_registry parameter, 4 shape test functions, helpers
- `packages/server-rust/src/service/domain/sync.rs` — Removed shape_merkle references
- `packages/server-rust/src/service/domain/query.rs` — Inlined project_fields(), removed shape references
- `packages/server-rust/src/network/module.rs` — Removed shape_registry: None
- `packages/server-rust/src/storage/query_merkle.rs` — Cleaned stale Shape comments
- `packages/client/src/SyncEngine.ts` — Removed subscribeShape() method
- `packages/client/src/sync/ClientMessageHandlers.ts` — Removed SHAPE_RESP/SHAPE_UPDATE handlers
- `packages/client/src/index.ts` — Removed ShapeHandle/ShapeUpdate re-exports
- `packages/client/src/sync/index.ts` — Removed ShapeManager re-export
- `packages/core/src/schemas/index.ts` — Removed shape schema re-exports
- `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts` — Updated message type count, removed shapeManager mocks
- `apps/docs-astro/src/content/docs/guides/live-queries.mdx` — Added fields, limit, Merkle reconnect docs
- `.specflow/todos/TODO.md` — Marked TODO-070 as superseded

**Removed:**
- `packages/core-rust/src/messages/shape.rs` — Shape message structs
- `packages/server-rust/src/service/domain/shape.rs` — ShapeService and ShapeRegistry
- `packages/server-rust/src/service/domain/shape_evaluator.rs` — Shape field evaluator
- `packages/server-rust/src/storage/shape_merkle.rs` — ShapeMerkleSyncManager
- `packages/client/src/ShapeHandle.ts` — Deprecated ShapeHandle class
- `packages/client/src/sync/ShapeManager.ts` — Deprecated ShapeManager class
- `packages/core/src/schemas/shape-schemas.ts` — Shape Zod schemas
- `tests/integration-rust/shape.test.ts` — Shape integration tests
- `apps/docs-astro/src/content/docs/guides/shapes.mdx` — Shapes guide page

### Deviations from Delta

- `packages/server-rust/src/storage/query_merkle.rs` — Not in Delta but modified to clean 6 stale Shape references in comments (needed to pass AC7 grep check)
- `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts` — Not in Delta but modified to fix CLIENT_MESSAGE_TYPES count (35→33) and remove stale shapeManager mocks

### Patterns Established

None — followed existing patterns.

### Spec Deviations

1. Fixed 2 CrdtService::new() test calls in crdt.rs that still passed `None` as 6th arg (shape_registry removal)
2. Updated CLIENT_MESSAGE_TYPES count test from 35 to 33, removed stale shapeManager mocks
3. Cleaned 6 stale Shape references in comments (query.rs, query_merkle.rs) to pass AC7 grep check
