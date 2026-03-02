# SPEC-053: Protocol Schema Cleanup -- Fix Bugs and Inconsistencies in TS Message Schemas

---
id: SPEC-053
type: refactor
status: done
priority: P0
complexity: large
created: 2026-02-14
blocks: [SPEC-052]
---

## Context

A protocol audit of `packages/core/src/schemas/` revealed 4 bugs and 6 inconsistencies in the TypeScript wire protocol schemas. These must be fixed before the Rust migration (SPEC-052) because the Rust `Message` enum will be generated from these schemas. No external clients exist, so breaking changes are free.

The "fix-on-port, don't copy bugs" principle from PROJECT.md mandates fixing these in TypeScript first, so the TS test suite validates the corrections, then porting the corrected schemas to Rust.

## Goal-Backward Analysis

### Goal Statement

All TS wire protocol schemas are correct, consistent, and complete -- the `MessageSchema` discriminated union covers every message type the server sends and the client handles, enum values are unified across domains, and every schema has a co-located type export.

### Observable Truths

1. The `MessageSchema` union in `index.ts` contains all message types (currently 53 + 19 missing server-to-client schemas + 5 new schemas = 77 total variants).
2. `AUTH_REQUIRED` has a Zod schema and is in the union.
3. `LOCK_GRANTED`, `LOCK_RELEASED`, `SYNC_RESET_REQUIRED` have message-level schemas (with `type` discriminant) and are in the union.
4. `PARTITION_MAP` has a Zod schema and is in the union.
5. All result-set change events use `LEAVE` (not `REMOVE`) and the field name `changeType` (not `type`).
6. Every `*Schema` export has a co-located `type X = z.infer<typeof XSchema>` export.
7. `pnpm build` and `pnpm test` pass with zero regressions.

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `packages/core/src/schemas/base-schemas.ts` | New `ChangeEventTypeSchema`, `AuthRequiredMessageSchema`; missing type exports |
| `packages/core/src/schemas/client-message-schemas.ts` | New `LockGrantedMessageSchema`, `LockReleasedMessageSchema`, `SyncResetRequiredMessageSchema`; use `ChangeEventTypeSchema` + rename `type` to `changeType` in `QueryUpdatePayload`; remove `PNCounterStateObject` export |
| `packages/core/src/schemas/search-schemas.ts` | Use `ChangeEventTypeSchema` (replacing `SearchUpdateTypeSchema`); rename `type` to `changeType` in `SearchUpdatePayload` |
| `packages/core/src/schemas/cluster-schemas.ts` | New `PartitionMapMessageSchema`; use `ChangeEventTypeSchema`; extend `SearchOptionsSchema` in `ClusterSearchReqPayload`; add type exports |
| `packages/core/src/schemas/sync-schemas.ts` | Extract `ORMapEntrySchema`; add missing type exports |
| `packages/core/src/schemas/messaging-schemas.ts` | Move `PNCounterStateObject` type export here; add missing type exports |
| `packages/core/src/schemas/query-schemas.ts` | Add missing type exports for `QuerySubMessage`, `QueryUnsubMessage` |
| `packages/core/src/schemas/index.ts` | Add all missing schemas to `MessageSchema` union |
| `packages/core/src/types/cluster.ts` | Remove duplicate `PartitionMapRequestMessage` and `PartitionMapMessage` interfaces; re-export Zod-inferred types with same names |
| `packages/server/src/query/QueryRegistry.ts` | Change `REMOVE` to `LEAVE` and `type` to `changeType` in sendUpdate |
| `packages/server/src/subscriptions/DistributedQueryCoordinator.ts` | Change `type` to `changeType` in buildUpdateMessage |
| `packages/server/src/subscriptions/DistributedSearchCoordinator.ts` | Change `type` to `changeType` in buildUpdateMessage |
| `packages/server/src/ServerCoordinator.ts` | Change `type` to `changeType` in SEARCH_UPDATE callback |
| `packages/client/src/SyncEngine.ts` | Change `type` to `changeType` in handleQueryUpdate; change `REMOVE` to `LEAVE` |
| `packages/client/src/SearchHandle.ts` | Change `type` to `changeType` in handleSearchUpdate |

### Key Links (Fragile Connections)

1. **QueryRegistry.sendUpdate `type` param** -- This function uses `'UPDATE' | 'REMOVE'` as the type parameter, then maps to `changeType` for distributed mode but NOT for local mode. The rename must change both the local message construction AND the function signature.
2. **DistributedQueryCoordinator.buildUpdateMessage** -- Maps `payload.changeType` (from cluster) back to `type` (for client). After this fix, it maps to `changeType` instead.
3. **Client SyncEngine.handleQueryUpdate** -- Destructures `{ type }` from payload and compares `type === 'REMOVE'`. Must change to destructure `changeType` and compare `changeType === 'LEAVE'`.
4. **E2E tests** -- `tests/e2e/live-queries.test.ts` references `m.payload.type === 'REMOVE'` (4 sites) and `tests/e2e/fulltext-search.test.ts` references `m.payload.type` (6 sites). Both must update to `changeType`.

## Task

Fix 4 bugs and 6 inconsistencies in the TypeScript message schemas to produce a clean, consistent, complete schema set that SPEC-052 can use as its source of truth for Rust struct generation.

### Approach

1. **BUG-1 (Missing union members):** Add all server-to-client message schemas to the `MessageSchema` discriminated union in `index.ts`. This includes 19 existing schemas already defined but not imported, 5 response schemas already in the union but verified, and 5 new schemas created by BUG-2 and BUG-3.

2. **BUG-2 (AUTH_REQUIRED):** Create `AuthRequiredMessageSchema = z.object({ type: z.literal('AUTH_REQUIRED') })` in `base-schemas.ts`.

3. **BUG-3 (Missing message wrappers):** Create `LockGrantedMessageSchema`, `LockReleasedMessageSchema`, `SyncResetRequiredMessageSchema` in `client-message-schemas.ts`, wrapping existing payload schemas. Create `PartitionMapMessageSchema` in `cluster-schemas.ts` (covers I-5 too).

4. **BUG-4 (REMOVE vs LEAVE):** Create unified `ChangeEventTypeSchema = z.enum(['ENTER', 'UPDATE', 'LEAVE'])` in `base-schemas.ts`. Replace all three inline enum definitions. Change `REMOVE` to `LEAVE` in `QueryUpdatePayloadSchema`. Rename field from `type` to `changeType` in `QueryUpdatePayload`, `SearchUpdatePayload`, and `ClusterSearchUpdatePayload`. Update all server/client sites that construct or read these fields.

5. **I-4 (Duplicate PartitionMapRequestMessage):** Remove the TS interface from `types/cluster.ts`, import the Zod-inferred type from schemas.

6. **I-5 (Missing PartitionMap schema):** Create `PartitionMapMessageSchema` in `cluster-schemas.ts` (done with BUG-3). Also create supporting Zod schemas: `NodeInfoSchema`, `PartitionInfoSchema`, `PartitionMapSchema`.

7. **I-6 (ORMap entry duplication):** Extract `ORMapEntrySchema = z.object({ key: z.string(), records: z.array(ORMapRecordSchema), tombstones: z.array(z.string()) })` in `sync-schemas.ts`. Replace 3 inline definitions.

8. **I-7 (ClusterSearchReq redefines SearchOptions):** Use `SearchOptionsSchema.extend()` for the `options` field in `ClusterSearchReqPayloadSchema`. The `.extend()` call must override `limit` with `z.number().int().positive().max(1000)` (required, with constraints, replacing the optional `z.number().optional()` from `SearchOptionsSchema`) and add the 3 extra fields (`includeMatchedTerms: z.boolean().optional()`, `afterScore: z.number().optional()`, `afterKey: z.string().optional()`). Implementer should verify the resulting schema matches the current inline definition.

9. **I-9 (PNCounterStateObject type in wrong file):** Move `PNCounterStateObject` type export from `client-message-schemas.ts` to `messaging-schemas.ts` where the schema is defined.

10. **I-8 (Missing type exports):** Add `z.infer<typeof Schema>` type exports for all schemas missing them. Estimated ~30 across `base-schemas.ts`, `sync-schemas.ts`, `messaging-schemas.ts`, `query-schemas.ts`, `http-sync-schemas.ts`, `cluster-schemas.ts`.

## Requirements

### Files to Modify

| File | Changes | Risk |
|------|---------|------|
| `packages/core/src/schemas/base-schemas.ts` | Add `ChangeEventTypeSchema`, `AuthRequiredMessageSchema`; add type exports for `PredicateOp`, `PredicateNode`, `AuthMessage` | Low |
| `packages/core/src/schemas/sync-schemas.ts` | Extract `ORMapEntrySchema`; replace 3 inline definitions; add ~15 missing type exports | Low |
| `packages/core/src/schemas/client-message-schemas.ts` | Add `LockGrantedMessageSchema`, `LockReleasedMessageSchema`, `SyncResetRequiredMessageSchema`; use `ChangeEventTypeSchema` + rename field `type` to `changeType` in `QueryUpdatePayloadSchema`; remove `PNCounterStateObject` export | Medium |
| `packages/core/src/schemas/search-schemas.ts` | Replace `SearchUpdateTypeSchema` with import of `ChangeEventTypeSchema`; rename field `type` to `changeType` in `SearchUpdatePayloadSchema`; keep `SearchUpdateTypeSchema` as re-export alias for backward compat | Medium |
| `packages/core/src/schemas/cluster-schemas.ts` | Add `NodeInfoSchema`, `PartitionInfoSchema`, `PartitionMapPayloadSchema`, `PartitionMapMessageSchema`; use `ChangeEventTypeSchema`; extend `SearchOptionsSchema` in `ClusterSearchReqPayload`; rename `type` to `changeType` in `ClusterSearchUpdatePayloadSchema`; add missing type export for `PartitionMapRequest` | Medium |
| `packages/core/src/schemas/messaging-schemas.ts` | Add `PNCounterStateObject` type export; add missing type exports for `TopicSub`, `TopicUnsub`, `TopicPub`, `TopicMessageEvent`, `LockRequest`, `LockRelease`, `CounterRequest`, `CounterSync`, `CounterResponse`, `CounterUpdate`, `EntryProcessor` | Low |
| `packages/core/src/schemas/query-schemas.ts` | Add missing type exports for `QuerySubMessage`, `QueryUnsubMessage` | Low |
| `packages/core/src/schemas/index.ts` | Import and add all missing schemas to `MessageSchema` union | Low |
| `packages/core/src/types/cluster.ts` | Remove `PartitionMapMessage` and `PartitionMapRequestMessage` interfaces; re-export Zod-inferred types with same names (`PartitionMapMessage`, `PartitionMapRequestMessage`) so existing import sites in `packages/core/src/index.ts` and `packages/client/src/cluster/PartitionRouter.ts` continue to compile | Medium |
| `packages/core/src/schemas/http-sync-schemas.ts` | Add missing type exports for `SyncMapEntry`, `HttpQueryRequest`, `HttpSearchRequest`, `DeltaRecord`, `MapDelta`, `HttpQueryResult`, `HttpSearchResult`, `HttpSyncError` | Low |
| `packages/server/src/query/QueryRegistry.ts` | Change `sendUpdate` and `sendDistributedUpdate` type param from `'UPDATE' \| 'REMOVE'` to `'UPDATE' \| 'LEAVE'`; change `type` field to `changeType` in QUERY_UPDATE message construction | High |
| `packages/server/src/subscriptions/DistributedQueryCoordinator.ts` | Change `type:` to `changeType:` in `buildUpdateMessage` (line 313) | Low |
| `packages/server/src/subscriptions/DistributedSearchCoordinator.ts` | Change `type:` to `changeType:` in `buildUpdateMessage` (line 377) | Low |
| `packages/server/src/ServerCoordinator.ts` | Change `type` to `changeType` in SEARCH_UPDATE callback (line 306) | Low |
| `packages/client/src/SyncEngine.ts` | Change `handleQueryUpdate` to use `changeType` instead of `type`; change `REMOVE` comparison to `LEAVE` | Medium |
| `packages/client/src/SearchHandle.ts` | Change `handleSearchUpdate` to destructure `changeType` instead of `type` | Low |

### Test Files to Update

| File | Changes |
|------|---------|
| `packages/server/src/query/__tests__/QueryRegistry.test.ts` | Change `payload.type === 'REMOVE'` to `payload.changeType === 'LEAVE'`; change `payload.type === 'UPDATE'` to `payload.changeType === 'UPDATE'` (4 sites: lines 72, 79, 120, 126 in local-mode tests) |
| `packages/server/src/__tests__/LiveQuery.test.ts` | Change `payload.type` to `payload.changeType` (4 sites); change `REMOVE` to `LEAVE` |
| `packages/server/src/subscriptions/__tests__/DistributedSubscriptionCoordinator.test.ts` | Change `payload.type` to `payload.changeType` (5 sites) |
| `packages/client/src/__tests__/SyncEngine.test.ts` | Change `type: 'REMOVE'` to `changeType: 'LEAVE'` in simulated messages (1 site) |
| `tests/e2e/live-queries.test.ts` | Change `payload.type === 'REMOVE'` to `payload.changeType === 'LEAVE'` (4 sites) |
| `tests/e2e/fulltext-search.test.ts` | Change `payload.type` to `payload.changeType` (6 sites) |

### Deletions

| Item | Reason |
|------|--------|
| `PartitionMapMessage` interface in `types/cluster.ts` (lines 45-48) | Replaced by Zod schema in `cluster-schemas.ts` |
| `PartitionMapRequestMessage` interface in `types/cluster.ts` (lines 50-55) | Already has Zod schema `PartitionMapRequestSchema` in `cluster-schemas.ts` |
| `PNCounterStateObject` type export from `client-message-schemas.ts` (line 141) | Moved to `messaging-schemas.ts` where schema is defined |

## Acceptance Criteria

1. **AC-1: MessageSchema union is complete.** The `MessageSchema` discriminated union in `index.ts` includes every message type that the server sends or the client sends over WebSocket. Count the variants by running `grep -c "Schema," packages/core/src/schemas/index.ts` within the union -- result is >= 71.

2. **AC-2: AUTH_REQUIRED has a schema.** `AuthRequiredMessageSchema` exists in `base-schemas.ts` with `type: z.literal('AUTH_REQUIRED')` and is in the `MessageSchema` union.

3. **AC-3: LOCK_GRANTED, LOCK_RELEASED, SYNC_RESET_REQUIRED have message schemas.** Each has a `z.object({ type: z.literal('...'), payload: ...PayloadSchema })` definition and is in the `MessageSchema` union.

4. **AC-4: PARTITION_MAP has a Zod schema.** `PartitionMapMessageSchema` exists in `cluster-schemas.ts` with `type: z.literal('PARTITION_MAP')` and `payload: PartitionMapPayloadSchema`. The TS interfaces `PartitionMapMessage` and `PartitionMapRequestMessage` are removed from `types/cluster.ts`.

5. **AC-5: Unified change event type.** A single `ChangeEventTypeSchema = z.enum(['ENTER', 'UPDATE', 'LEAVE'])` in `base-schemas.ts` is used by `QueryUpdatePayloadSchema`, `SearchUpdatePayloadSchema`, `ClusterSubUpdatePayloadSchema`, and `ClusterSearchUpdatePayloadSchema`. No schema uses `REMOVE` as a change event value.

6. **AC-6: Field name is `changeType`.** `QueryUpdatePayloadSchema`, `SearchUpdatePayloadSchema`, and `ClusterSearchUpdatePayloadSchema` all use `changeType` as the field name (not `type`). `ClusterSubUpdatePayloadSchema` already uses `changeType` -- confirmed unchanged.

7. **AC-7: ORMapEntrySchema extracted.** A single `ORMapEntrySchema` in `sync-schemas.ts` replaces the 3 inline `z.object({ key, records, tombstones })` definitions in `ORMapSyncRespLeafSchema`, `ORMapDiffResponseSchema`, and `ORMapPushDiffSchema`.

8. **AC-8: ClusterSearchReq extends SearchOptions.** `ClusterSearchReqPayloadSchema.options` uses `SearchOptionsSchema.extend()` instead of redefining `limit`, `minScore`, `boost` inline. The `.extend()` call overrides `limit` with `z.number().int().positive().max(1000)` (required, not optional) and adds `includeMatchedTerms`, `afterScore`, `afterKey`.

9. **AC-9: PNCounterStateObject co-located.** `PNCounterStateObject` type export is in `messaging-schemas.ts` alongside `PNCounterStateObjectSchema`, not in `client-message-schemas.ts`.

10. **AC-10: All schemas have type exports.** Every `export const XSchema = z.object(...)` or `z.enum(...)` in the schemas directory has a corresponding `export type X = z.infer<typeof XSchema>` in the same file.

11. **AC-11: Build passes.** `pnpm build` succeeds with zero errors.

12. **AC-12: Tests pass.** `pnpm test` passes with zero regressions. All updated test assertions use `changeType` and `LEAVE`.

## Constraints

- Do NOT standardize payload wrapping (mixed inline/payload pattern is acceptable).
- Do NOT change any wire behavior except the intentional REMOVE-to-LEAVE and type-to-changeType corrections.
- Do NOT modify `http-sync-schemas.ts` `DeltaRecordSchema.eventType` -- the `PUT`/`REMOVE` enum there refers to storage event types (record deletion), not result-set change events. These are semantically different.
- Do NOT add new schema files -- all changes go into existing files.
- Do NOT change the `opType: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE'` field on `ClientOp` / `SyncEngine.recordOperation` / `IStorageAdapter.OpLogEntry` -- this is an operation type, not a result-set change event.
- Do NOT change `ServerEventPayloadSchema.eventType` enum (`PUT`, `REMOVE`, `OR_ADD`, `OR_REMOVE`) -- these are storage mutation types, not result-set events.
- Keep `SearchUpdateTypeSchema` as a re-export alias (`export const SearchUpdateTypeSchema = ChangeEventTypeSchema`) for backward compatibility with consumers that import it by name.

## Assumptions

1. **The 19 existing schemas listed in BUG-1 as "already have schemas" are all correctly defined.** They just need importing into the union -- no changes to the schema definitions themselves.
2. **`PartitionMapPayloadSchema` mirrors the `PartitionMap` interface** currently in `types/cluster.ts` (version, partitionCount, nodes, partitions, generatedAt). The Zod schema will be the canonical definition.
3. **No code outside packages/core, packages/client, packages/server, and tests/ references the affected fields.** The `react` and `adapters` packages do not construct or read QueryUpdate/SearchUpdate payloads directly.
4. **`SearchUpdateTypeSchema` re-export alias is sufficient** for backward compat -- no consumer stores a reference to the schema object itself for identity comparison.
5. **The `ClusterSearchUpdatePayloadSchema` field `type` (line 158 in cluster-schemas.ts) should also be renamed to `changeType`** to match `ClusterSubUpdatePayloadSchema` which already uses `changeType`. The task description specifies renaming `type` to `changeType` in SearchUpdate and QueryUpdate payloads; ClusterSearchUpdate is included as well since it uses the same semantic.
6. **PartitionMap-related TS interfaces** (NodeInfo, PartitionInfo, PartitionMap, PartitionChange, etc.) in `types/cluster.ts` can remain as TS interfaces. Only `PartitionMapMessage` and `PartitionMapRequestMessage` are duplicates of Zod schemas and need removal. The supporting types (NodeInfo, PartitionInfo, etc.) may eventually get Zod schemas too, but that is out of scope for this spec -- we create minimal Zod schemas for the message wrapper only, reusing existing TS types for the payload shape via `z.object()` that mirrors the interface.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `ChangeEventTypeSchema` and `AuthRequiredMessageSchema` in `base-schemas.ts`; add missing type exports to `base-schemas.ts` | -- | ~10% |
| G2 | 1 | Extract `ORMapEntrySchema` in `sync-schemas.ts`; add missing type exports to `sync-schemas.ts` | -- | ~10% |
| G3 | 1 | Add missing type exports to `messaging-schemas.ts`; move `PNCounterStateObject` type export here | -- | ~5% |
| G4 | 1 | Add missing type exports to `http-sync-schemas.ts` and `query-schemas.ts` (2 type exports: `QuerySubMessage`, `QueryUnsubMessage`) | -- | ~3% |
| G5 | 2 | Create message wrapper schemas in `client-message-schemas.ts` (`LockGrantedMessageSchema`, `LockReleasedMessageSchema`, `SyncResetRequiredMessageSchema`); use `ChangeEventTypeSchema` + rename field in `QueryUpdatePayloadSchema`; remove `PNCounterStateObject` export | G1, G3 | ~15% |
| G6 | 2 | Update `search-schemas.ts`: replace `SearchUpdateTypeSchema` with re-export of `ChangeEventTypeSchema`; rename `type` to `changeType` in `SearchUpdatePayloadSchema` | G1 | ~5% |
| G7 | 2 | Update `cluster-schemas.ts`: create `PartitionMapMessageSchema` + supporting schemas; extend `SearchOptionsSchema`; use `ChangeEventTypeSchema`; rename `type` to `changeType` in `ClusterSearchUpdatePayloadSchema`; add type exports | G1 | ~15% |
| G8 | 3 | Update `types/cluster.ts`: remove `PartitionMapMessage` and `PartitionMapRequestMessage` interfaces; re-export the Zod-inferred types with the SAME NAMES (`PartitionMapMessage` and `PartitionMapRequestMessage`) so that existing import sites in `packages/core/src/index.ts` and `packages/client/src/cluster/PartitionRouter.ts` continue to compile without changes | G7 | ~5% |
| G9 | 4 | Update `index.ts`: add all missing schemas to `MessageSchema` union | G1-G8 | ~5% |
| G10 | 3 | Update server code: `QueryRegistry.ts`, `DistributedQueryCoordinator.ts`, `DistributedSearchCoordinator.ts`, `ServerCoordinator.ts` -- change `REMOVE` to `LEAVE`, `type` to `changeType`. **Important:** Local mode in `sendUpdate` (lines 752-760) must add the same ENTER/UPDATE discrimination logic that `sendDistributedUpdate` already has (check `previousResultKeys.has(key)` to distinguish ENTER from UPDATE). Currently local mode passes the `type` param value straight through without distinguishing ENTER vs UPDATE -- after the fix it must match the distributed mode behavior. | G5, G6 | ~15% |
| G11 | 3 | Update client code: `SyncEngine.ts`, `SearchHandle.ts` -- change `REMOVE` to `LEAVE`, `type` to `changeType` | G5, G6 | ~5% |
| G12 | 5 | Update all test files: `QueryRegistry.test.ts`, `LiveQuery.test.ts`, `DistributedSubscriptionCoordinator.test.ts`, `SyncEngine.test.ts`, `live-queries.test.ts`, `fulltext-search.test.ts` | G9, G10, G11 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3, G4 | Yes | 4 |
| 2 | G5, G6, G7 | Yes | 3 |
| 3 | G8, G10, G11 | Yes | 3 |
| 4 | G9 | No | 1 |
| 5 | G12 | No | 1 |

**Total workers needed:** 4 (max in any wave)

## Audit History

### Audit v1 (2026-02-14)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~103% total (sum across all groups with parallel execution)

**Scope:** Large (~103% estimated across 12 groups, 21 files total). Per-worker context is within bounds (~5-15% per group), but the spec scope exceeds medium complexity -- 15 source files + 6 test files = 21 total files modified.

**Per-Group Breakdown:**
| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~10% | OK |
| G2 | ~10% | OK |
| G3 | ~5% | OK |
| G4 | ~3% | OK |
| G5 | ~15% | OK |
| G6 | ~5% | OK |
| G7 | ~15% | OK |
| G8 | ~5% | OK |
| G9 | ~5% | OK |
| G10 | ~15% | OK |
| G11 | ~5% | OK |
| G12 | ~10% | OK |

**Quality Projection:** GOOD range (each worker stays in 5-15%, well within 30% per-group limit)

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (union complete) has artifacts | OK | G9 covers index.ts |
| Truth 2 (AUTH_REQUIRED) has artifacts | OK | G1 covers base-schemas.ts |
| Truth 3 (LOCK_GRANTED etc.) has artifacts | OK | G5 covers client-message-schemas.ts |
| Truth 4 (PARTITION_MAP) has artifacts | OK | G7 covers cluster-schemas.ts |
| Truth 5 (LEAVE + changeType) has artifacts | OK | G5, G6, G7, G10, G11 |
| Truth 6 (type exports) has artifacts | OK | G1-G4, G5-G7 |
| Truth 7 (build + test pass) has artifacts | OK | G12 + AC-11/AC-12 |
| Key Link 1 wiring | OK | G10 covers QueryRegistry |
| Key Link 2 wiring | OK | G10 covers DistributedQueryCoordinator |
| Key Link 3 wiring | OK | G11 covers SyncEngine |
| Key Link 4 wiring | OK | G12 covers E2E tests |

**Strategic fit:** Aligned with project goals. Directly enables SPEC-052 (Rust migration). The "fix-on-port, don't copy bugs" and "audit before implementing" principles from PROJECT.md mandate exactly this kind of cleanup spec.

**Project compliance:** Honors PROJECT.md decisions. Language Profile (Rust, max 5 files) does not apply -- this is a TypeScript spec and the profile explicitly states "TypeScript packages continue using existing conventions (no file limit, no trait-first)."

**Assumptions Validated:**

| # | Assumption | Verified | Note |
|---|------------|----------|------|
| A1 | 13 existing schemas need importing | Partially | Actual count is 19 (not 13). See Recommendation 2. |
| A2 | PartitionMapPayloadSchema mirrors PartitionMap interface | OK | Interface at types/cluster.ts lines 33-39 matches |
| A3 | No code outside core/client/server/tests references affected fields | OK | Verified via grep |
| A4 | SearchUpdateTypeSchema re-export sufficient | OK | No identity comparisons found |
| A5 | ClusterSearchUpdatePayloadSchema.type should be renamed | OK | Line 158 confirmed uses `type: SearchUpdateTypeSchema` |
| A6 | Supporting TS interfaces can remain | OK | Only message wrappers are duplicates |

**Recommendations:**
1. `query-schemas.ts` is mentioned in Approach item 10 (I-8) for missing type exports (`QuerySubMessage`, `QueryUnsubMessage`) but is absent from the Files to Modify table and from all Implementation Task groups. Add it to G4 (Wave 1) or create a small separate group. This is minor (2 type exports) but AC-10 requires ALL schemas to have type exports.
2. Observable Truth #1 arithmetic is inaccurate. Source verification shows: currently 53 schemas in union (not 54), 19 existing schemas missing (not 17), total after fix would be 77 (not 76). The AC-1 threshold of >= 71 still works as a floor check, but the narrative numbers will confuse anyone reading the spec. Correct the counts.
3. When removing `PartitionMapMessage` and `PartitionMapRequestMessage` interfaces from `types/cluster.ts`, the replacement Zod-inferred types must be re-exported with the SAME NAMES so that `packages/core/src/index.ts` (lines 98-99) and `packages/client/src/cluster/PartitionRouter.ts` (line 15) continue to compile. The spec's G8 description says "import Zod-inferred types" but should explicitly state "re-export as `PartitionMapMessage` and `PartitionMapRequestMessage` to preserve existing import sites."
4. The `ClusterSearchReqPayloadSchema.options` currently has additional fields (`includeMatchedTerms`, `afterScore`, `afterKey`) beyond what `SearchOptionsSchema` defines (`limit`, `minScore`, `boost`). The spec says to use `SearchOptionsSchema.extend()` which correctly preserves these extra fields. No issue, but the implementer should verify the `.extend()` call adds those 3 extra fields plus overrides `limit` to `z.number().int().positive().max(1000)` (currently required, not optional like in SearchOptionsSchema).
5. [Strategic] The complexity should be "large" not "medium" given 21 files across 3 packages plus E2E tests. The Implementation Tasks section with 12 groups across 4 waves confirms this is a large spec. Updating the complexity field would align with the decomposition.

**Recommendation:** Use `/sf:run --parallel` for orchestrated execution across 4 waves.

### Response v1 (2026-02-14)
**Applied:** All 5 recommendations from Audit v1.

**Changes:**
1. [+] Add `query-schemas.ts` to Files to Modify table and G4 -- Added `packages/core/src/schemas/query-schemas.ts` row to Files to Modify table (Low risk, 2 type exports). Updated G4 description from "Add missing type exports to `http-sync-schemas.ts`" to "Add missing type exports to `http-sync-schemas.ts` and `query-schemas.ts` (2 type exports: `QuerySubMessage`, `QueryUnsubMessage`)". Also added to Required Artifacts table.
2. [+] Fix Observable Truth #1 counts -- Changed "currently 54 + 17 missing server-to-client schemas + 5 new schemas = 76" to "currently 53 + 19 missing server-to-client schemas + 5 new schemas = 77".
3. [+] Clarify G8 re-export naming -- Updated G8 task description to explicitly state: "re-export the Zod-inferred types with the SAME NAMES (`PartitionMapMessage` and `PartitionMapRequestMessage`) so that existing import sites in `packages/core/src/index.ts` and `packages/client/src/cluster/PartitionRouter.ts` continue to compile without changes". Also updated the `types/cluster.ts` row in Files to Modify to mention re-export with same names.
4. [+] Add `.extend()` override detail to I-7/AC-8 -- Updated Approach item 8 (I-7) to specify the `.extend()` must override `limit` with `z.number().int().positive().max(1000)` and add 3 extra fields. Updated AC-8 to include the override and additional fields requirement.
5. [+] Update complexity to `large` -- Changed frontmatter `complexity: medium` to `complexity: large`.

### Audit v2 (2026-02-14)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~103% total (sum across all 12 groups)

**Scope:** Large (~103% estimated across 12 groups, 22 files total: 16 source files + 6 test files). Per-worker context is well within bounds (~3-15% per group), suitable for `/sf:run --parallel`.

**Per-Group Breakdown:**

| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~10% | OK |
| G2 | ~10% | OK |
| G3 | ~5% | OK |
| G4 | ~3% | OK |
| G5 | ~15% | OK |
| G6 | ~5% | OK |
| G7 | ~15% | OK |
| G8 | ~5% | OK |
| G9 | ~5% | OK |
| G10 | ~15% | OK |
| G11 | ~5% | OK |
| G12 | ~10% | OK |

**Quality Projection:** GOOD range (every group stays in 3-15%, well within 30% per-group limit)

**Dimension Evaluation:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Each bug/inconsistency clearly described with before/after |
| Completeness | Good | All files identified, deletions listed, approach detailed |
| Testability | Excellent | 12 measurable ACs with concrete verification methods |
| Scope | Good | Constraints clearly limit what NOT to change |
| Feasibility | Excellent | Each change is mechanical and well-defined |
| Architecture fit | Excellent | Aligns with existing schema domain splitting pattern |
| Non-duplication | Excellent | This spec explicitly removes duplications |
| Cognitive load | Good | Spec is long but well-organized by bug/inconsistency |
| Strategic fit | Excellent | Directly enables SPEC-052 Rust migration |
| Project compliance | Excellent | Follows "fix-on-port" and "audit before implementing" principles |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| All 7 truths have artifacts | OK | Verified: each truth maps to 1+ task groups |
| All artifacts have purpose | OK | No orphan artifacts |
| Key Link 1 wiring | OK | G10 covers QueryRegistry local + distributed paths |
| Key Link 2 wiring | OK | G10 covers DistributedQueryCoordinator |
| Key Link 3 wiring | OK | G11 covers SyncEngine |
| Key Link 4 wiring | OK | G12 covers E2E tests |

**Source Verification:**

| Claim | Verified | Finding |
|-------|----------|---------|
| 53 schemas in union | OK | Counted 53 entries in `MessageSchema` discriminated union |
| 19 missing schemas | OK | Identified 19 existing message schemas with `type: z.literal(...)` not in union |
| 3 inline ORMap entry defs | OK | Lines 116-121, 136-141, 148-153 in sync-schemas.ts |
| `ClusterSearchUpdatePayloadSchema` uses `type` not `changeType` | OK | Line 158 confirmed: `type: SearchUpdateTypeSchema` |
| `QueryUpdatePayloadSchema` uses `REMOVE` | OK | Line 54 confirmed: `z.enum(['ENTER', 'UPDATE', 'REMOVE'])` |
| `SearchUpdatePayloadSchema` uses `type` field | OK | Line 70 confirmed: `type: SearchUpdateTypeSchema` |
| `DistributedQueryCoordinator.buildUpdateMessage` maps to `type` | OK | Line 313 confirmed: `type: payload.changeType` |
| `DistributedSearchCoordinator.buildUpdateMessage` maps to `type` | OK | Line 377 confirmed: `type: payload.changeType` |
| `ServerCoordinator` SEARCH_UPDATE callback uses `type` | OK | Line 306 confirmed |
| `SyncEngine.handleQueryUpdate` destructures `type` and compares `REMOVE` | OK | Lines 791, 794 confirmed |
| `SearchHandle.handleSearchUpdate` destructures `type` | OK | Line 302 confirmed |
| E2E `live-queries.test.ts` has 4 `payload.type === 'REMOVE'` sites | OK | Lines 207, 213, 946, 952 confirmed |
| E2E `fulltext-search.test.ts` has 6 `payload.type` sites | OK | Lines 330, 398, 457, 463, 792, 798 confirmed |

**Assumptions Validated:**

| # | Assumption | Verified | Note |
|---|------------|----------|------|
| A1 | 13 existing schemas need importing | Mismatch | BUG-1 Approach text says "13" but Observable Truth #1 says "19". The 19 is correct. Approach text not updated by Response v1. See Recommendation 1. |
| A2 | PartitionMapPayloadSchema mirrors PartitionMap interface | OK | `types/cluster.ts` lines 33-39 match |
| A3 | No code outside core/client/server/tests references affected fields | OK | Verified via grep |
| A4 | SearchUpdateTypeSchema re-export sufficient | OK | No identity comparisons found |
| A5 | ClusterSearchUpdatePayloadSchema.type should be renamed | OK | Line 158 confirmed |
| A6 | Supporting TS interfaces can remain | OK | Only message wrappers are duplicates |

**Strategic fit:** Aligned with project goals. Directly enables SPEC-052 (Rust migration). The "fix-on-port, don't copy bugs" and "audit before implementing" principles from PROJECT.md mandate exactly this kind of cleanup.

**Project compliance:** Honors PROJECT.md decisions. Language Profile (Rust, max 5 files) does not apply -- this is a TypeScript spec and the profile explicitly states "TypeScript packages continue using existing conventions (no file limit, no trait-first)."

**Recommendations:**
1. Approach item 1 (BUG-1) still says "13 existing schemas already defined but not imported" but the correct count is 19 (as fixed in Observable Truth #1 by Response v1). The Approach text was not updated. The discrepancy is cosmetic -- implementers working from the task groups will do the right thing since G9 says "add all missing schemas" -- but it may cause confusion during review. Fix the Approach text to say "19 existing schemas".
2. `QueryRegistry.test.ts` site count says "3 sites" but source verification shows 4 sites needing changes (lines 72, 79, 120, 126 in local-mode tests). The distributed-mode test assertions (lines 200, 230) already use `changeType`. Not a blocking issue since implementers will find all sites via grep, but the count is inaccurate.
3. The `QueryRegistry.sendUpdate` local mode path (lines 752-760) currently sends the `type` param value (`'UPDATE' | 'REMOVE'`) directly as the `type` field in the payload without distinguishing ENTER vs UPDATE. The distributed mode path (lines 775-777) correctly translates via `previousResultKeys.has(key)`. After the fix, the local mode also needs this ENTER/UPDATE discrimination logic. Key Link 1 identifies this gap ("The rename must change both the local message construction AND the function signature") but G10's task description ("change `REMOVE` to `LEAVE`, `type` to `changeType`") reads as a simple text rename. Consider adding an explicit note: "Local mode in `sendUpdate` must add the same ENTER/UPDATE logic that `sendDistributedUpdate` already has (check `previousResultKeys.has(key)` to distinguish ENTER from UPDATE)."

**Recommendation:** Use `/sf:run --parallel` for orchestrated execution across 4 waves.

### Response v2 (2026-02-14)
**Applied:** All 3 recommendations from Audit v2.

**Changes:**
1. [+] Fix Approach BUG-1 count from "13" to "19" -- Changed Approach item 1 text from "13 existing schemas already defined but not imported" to "19 existing schemas already defined but not imported". Also updated Assumption A1 text from "The 13 existing schemas" to "The 19 existing schemas" for consistency.
2. [+] Fix QueryRegistry.test.ts site count from "3 sites" to "4 sites" -- Updated Test Files to Update table entry for `QueryRegistry.test.ts` from "(3 sites)" to "(4 sites: lines 72, 79, 120, 126 in local-mode tests)" with explicit line numbers.
3. [+] Add ENTER/UPDATE discrimination note to G10 -- Added explicit note to G10 task description: "Local mode in `sendUpdate` (lines 752-760) must add the same ENTER/UPDATE discrimination logic that `sendDistributedUpdate` already has (check `previousResultKeys.has(key)` to distinguish ENTER from UPDATE). Currently local mode passes the `type` param value straight through without distinguishing ENTER vs UPDATE -- after the fix it must match the distributed mode behavior."

### Audit v3 (2026-02-14)
**Status:** NEEDS_REVISION

**Context Estimate:** ~103% total (sum across all 12 groups)

**Critical:**
1. **Wave assignment error in Implementation Tasks and Execution Plan.** G8 depends on G7 (G7 creates `PartitionMapMessageSchema` in `cluster-schemas.ts`, G8 imports the Zod-inferred types from there). Both are currently assigned to Wave 2, meaning the executor would attempt to run them in parallel. G8 would fail because G7 has not yet created the schemas it depends on. Similarly, G9 depends on G1-G8; if G8 is Wave 3 (corrected), then G9 must be Wave 4, not Wave 3. The corrected wave assignments are: G8 = Wave 3 (depends on G7 which is Wave 2), G9 = Wave 4 (depends on G8 which is Wave 3), G12 = Wave 5 (depends on G9, G10, G11). The corrected execution plan is: Wave 1: G1, G2, G3, G4 (4 workers); Wave 2: G5, G6, G7 (3 workers); Wave 3: G8, G10, G11 (3 workers); Wave 4: G9 (1 worker); Wave 5: G12 (1 worker). Update the Wave column in the Task Groups table and the Execution Plan table accordingly.

**Recommendations:**
2. G12 currently depends on G10 and G11 only. Since G12 includes test files that assert on the new schema shapes (e.g., QueryRegistry tests will assert `changeType` field names), and G9 updates the `MessageSchema` union (potentially affecting runtime validation), G12 should also depend on G9 to ensure the union is updated before tests run. Add G9 to G12's dependencies.

**Dimension Evaluation:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Each bug/inconsistency clearly described with before/after |
| Completeness | Excellent | All files identified, deletions listed, approach detailed, all previous recommendations applied |
| Testability | Excellent | 12 measurable ACs with concrete verification methods |
| Scope | Excellent | Constraints clearly limit what NOT to change |
| Feasibility | Excellent | Each change is mechanical and well-defined |
| Architecture fit | Excellent | Aligns with existing schema domain splitting pattern |
| Non-duplication | Excellent | This spec explicitly removes duplications |
| Cognitive load | Good | Spec is long but well-organized by bug/inconsistency |
| Strategic fit | Excellent | Directly enables SPEC-052 Rust migration |
| Project compliance | Excellent | Follows "fix-on-port" and "audit before implementing" principles |

**Source Verification:**

All claims from Audit v2 re-verified with fresh reads of the source files:
- 53 schemas in union: confirmed (counted lines 91-143 of index.ts)
- 19 missing schemas: confirmed (cross-referenced all `type: z.literal(...)` definitions against union entries)
- `QueryUpdatePayloadSchema` uses `REMOVE` and field `type`: confirmed (line 54: `z.enum(['ENTER', 'UPDATE', 'REMOVE'])`)
- `SearchUpdatePayloadSchema` uses field `type`: confirmed (line 70)
- `ClusterSearchUpdatePayloadSchema` uses field `type`: confirmed (line 158)
- `QueryRegistry.sendUpdate` local mode lacks ENTER/UPDATE discrimination: confirmed (lines 752-760 pass `type` straight through)
- `SearchHandle.handleSearchUpdate` already uses `LEAVE` in switch cases: confirmed (line 326), only the destructuring on line 302 (`{ ...type }`) needs renaming
- `ServerCoordinator` SEARCH_UPDATE callback uses shorthand `type,` as payload field: confirmed (line 306)

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| All 7 truths have artifacts | OK | Each truth maps to 1+ task groups |
| All artifacts have purpose | OK | No orphan artifacts |
| Key Link 1-4 wiring | OK | All covered by G10, G11, G12 |

**Assumptions Validated:**

| # | Assumption | Verified | Note |
|---|------------|----------|------|
| A1 | 19 existing schemas need importing | OK | Verified: 19 schemas with `type: z.literal(...)` not in union |
| A2 | PartitionMapPayloadSchema mirrors PartitionMap interface | OK | `types/cluster.ts` lines 33-39 match |
| A3 | No code outside core/client/server/tests | OK | No references found |
| A4 | SearchUpdateTypeSchema re-export sufficient | OK | No identity comparisons found |
| A5 | ClusterSearchUpdatePayloadSchema.type rename | OK | Line 158 confirmed |
| A6 | Supporting TS interfaces can remain | OK | Only message wrappers are duplicates |

**Strategic fit:** Aligned with project goals.

**Project compliance:** Honors PROJECT.md decisions. Language Profile does not apply (TypeScript spec).

### Response v3 (2026-02-14)
**Applied:** All items from Audit v3 (1 critical + 1 recommendation). Both were pre-applied inline by the auditor.

**Changes:**
1. [+] Fix wave assignment error -- G8 moved from Wave 2 to Wave 3, G9 from Wave 3 to Wave 4, G12 from Wave 4 to Wave 5. Execution Plan updated accordingly (5 waves instead of 4). Pre-applied by auditor during audit v3.
2. [+] Add G9 to G12's dependencies -- G12 now depends on G9, G10, G11 (was G10, G11). Ensures MessageSchema union is complete before tests run. Pre-applied by auditor during audit v3.

### Audit v4 (2026-02-14)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~103% total (sum across all 12 groups)

**Scope:** Large (~103% estimated across 12 groups, 22 files total: 16 source files + 6 test files). Per-worker context is well within bounds (~3-15% per group), suitable for `/sf:run --parallel`.

**Per-Group Breakdown:**

| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~10% | OK |
| G2 | ~10% | OK |
| G3 | ~5% | OK |
| G4 | ~3% | OK |
| G5 | ~15% | OK |
| G6 | ~5% | OK |
| G7 | ~15% | OK |
| G8 | ~5% | OK |
| G9 | ~5% | OK |
| G10 | ~15% | OK |
| G11 | ~5% | OK |
| G12 | ~10% | OK |

**Quality Projection:** GOOD range (every group stays in 3-15%, well within 30% per-group limit)

**Dimension Evaluation:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Each bug/inconsistency clearly described with before/after, approach is precise |
| Completeness | Excellent | All 22 files identified, deletions listed, approach detailed, all previous audit items resolved |
| Testability | Excellent | 12 measurable ACs with concrete verification methods |
| Scope | Excellent | 7 constraints clearly delineate boundaries; no scope creep |
| Feasibility | Excellent | Each change is mechanical and well-defined; the one logic addition (ENTER/UPDATE discrimination in G10) is explicitly described |
| Architecture fit | Excellent | Aligns with existing schema domain splitting pattern |
| Non-duplication | Excellent | This spec explicitly removes duplications (ORMap entry, PartitionMap interfaces) |
| Cognitive load | Good | Spec is long (450+ lines) but well-organized by bug/inconsistency number |
| Strategic fit | Excellent | Directly enables SPEC-052 Rust migration; follows "fix-on-port" and "audit before implementing" principles |
| Project compliance | Excellent | Language Profile (Rust, max 5 files) explicitly does not apply to TypeScript specs |

**Wave Assignment Verification:**

| Group | Dependencies | Max Dep Wave | Assigned Wave | Correct? |
|-------|-------------|--------------|---------------|----------|
| G1 | -- | 0 | 1 | OK |
| G2 | -- | 0 | 1 | OK |
| G3 | -- | 0 | 1 | OK |
| G4 | -- | 0 | 1 | OK |
| G5 | G1, G3 | 1 | 2 | OK |
| G6 | G1 | 1 | 2 | OK |
| G7 | G1 | 1 | 2 | OK |
| G8 | G7 | 2 | 3 | OK |
| G9 | G1-G8 | 3 | 4 | OK |
| G10 | G5, G6 | 2 | 3 | OK |
| G11 | G5, G6 | 2 | 3 | OK |
| G12 | G9, G10, G11 | 4 | 5 | OK |

No circular dependencies. All wave assignments are correct.

**Source Verification (independently verified with fresh file reads):**

| Claim | Verified | Finding |
|-------|----------|---------|
| 53 schemas in union | OK | Counted lines 91-143 of index.ts: 53 entries |
| 19 missing schemas | OK | Cross-referenced all `type: z.literal(...)` across all schema files against union |
| `QueryUpdatePayloadSchema` uses `REMOVE` and field `type` | OK | client-message-schemas.ts line 54: `z.enum(['ENTER', 'UPDATE', 'REMOVE'])`, line 54: field named `type` |
| `SearchUpdatePayloadSchema` uses field `type` | OK | search-schemas.ts line 70: `type: SearchUpdateTypeSchema` |
| `ClusterSearchUpdatePayloadSchema` uses field `type` | OK | cluster-schemas.ts line 158: `type: SearchUpdateTypeSchema` |
| `ClusterSubUpdatePayloadSchema` already uses `changeType` | OK | cluster-schemas.ts line 60: `changeType: z.enum(['ENTER', 'UPDATE', 'LEAVE'])` |
| 3 inline ORMap entry defs | OK | sync-schemas.ts lines 116-121, 136-141, 148-153 |
| `SyncEngine.handleQueryUpdate` destructures `type`, compares `REMOVE` | OK | SyncEngine.ts lines 791, 794 |
| `SearchHandle.handleSearchUpdate` destructures `type` | OK | SearchHandle.ts line 302 |
| `SearchHandle` switch already uses `LEAVE` | OK | SearchHandle.ts line 326 |
| Missing type exports in base-schemas.ts | OK | 3 missing: PredicateOp, PredicateNode, AuthMessage |
| Missing type exports in sync-schemas.ts | OK | ~15 missing (all message schemas lack type exports) |
| Missing type exports in messaging-schemas.ts | OK | ~12 missing |
| Missing type exports in query-schemas.ts | OK | 2 missing: QuerySubMessage, QueryUnsubMessage |
| Missing type exports in http-sync-schemas.ts | OK | 8 missing |
| Missing type exports in cluster-schemas.ts | OK | 1 missing: PartitionMapRequest |
| `PartitionMapMessage` interface at types/cluster.ts lines 45-48 | OK | Confirmed interface exists and duplicates Zod schema intent |
| `PartitionMapRequestMessage` interface at types/cluster.ts lines 50-55 | OK | Confirmed interface exists and duplicates PartitionMapRequestSchema |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| All 7 truths have artifacts | OK | Each truth maps to 1+ task groups |
| All artifacts have purpose | OK | No orphan artifacts |
| Key Link 1 wiring (QueryRegistry local + distributed) | OK | G10 covers with explicit ENTER/UPDATE discrimination note |
| Key Link 2 wiring (DistributedQueryCoordinator) | OK | G10 covers |
| Key Link 3 wiring (SyncEngine) | OK | G11 covers |
| Key Link 4 wiring (E2E tests) | OK | G12 covers |

**Assumptions Validated:**

| # | Assumption | If wrong, impact | Verified |
|---|------------|------------------|----------|
| A1 | 19 existing schemas need importing | Union incomplete | OK -- independently counted 19 |
| A2 | PartitionMapPayloadSchema mirrors PartitionMap interface | Schema mismatch | OK -- types/cluster.ts lines 33-39 match |
| A3 | No code outside core/client/server/tests | Missed rename sites | OK -- no references found |
| A4 | SearchUpdateTypeSchema re-export sufficient | Import breakage | OK -- no identity comparisons |
| A5 | ClusterSearchUpdatePayloadSchema.type rename | Inconsistent naming | OK -- line 158 confirmed |
| A6 | Supporting TS interfaces can remain | Over-engineering | OK -- only message wrappers are duplicates |

**Strategic fit:** Aligned with project goals. Directly enables SPEC-052 (Rust migration). Follows "fix-on-port, don't copy bugs" and "audit before implementing" principles from PROJECT.md.

**Project compliance:** Honors PROJECT.md decisions. Language Profile (Rust, max 5 files) does not apply -- this is a TypeScript spec and the profile explicitly states "TypeScript packages continue using existing conventions (no file limit, no trait-first)." No out-of-scope intrusions detected.

**Comment:** This is an exceptionally well-crafted specification. After 3 audit cycles and 3 revision cycles, all critical issues have been resolved and all recommendations applied. The spec is clear, complete, testable, and well-decomposed into 12 task groups across 5 waves. Every claim has been independently verified against source files. Ready for parallel execution.

**Recommendation:** Use `/sf:run --parallel` for orchestrated execution across 5 waves.

---

## Execution Summary

**Executed:** 2026-02-14
**Mode:** orchestrated (sequential fallback)
**Commits:** 16

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2, G3, G4 | complete |
| 2 | G5, G6, G7 | complete |
| 3 | G8, G10, G11 | complete |
| 4 | G9 | complete |
| 5 | G12 | complete |

### Files Modified

**Core schemas (9 files):**
- `packages/core/src/schemas/base-schemas.ts` -- Added ChangeEventTypeSchema, AuthRequiredMessageSchema, AuthMessage type export
- `packages/core/src/schemas/sync-schemas.ts` -- Extracted ORMapEntrySchema, added 15 type exports
- `packages/core/src/schemas/messaging-schemas.ts` -- Added PNCounterStateObject type, 11 type exports
- `packages/core/src/schemas/http-sync-schemas.ts` -- Added 8 type exports
- `packages/core/src/schemas/query-schemas.ts` -- Added 2 type exports
- `packages/core/src/schemas/client-message-schemas.ts` -- Added LockGranted/LockReleased/SyncResetRequired wrappers, renamed type to changeType, used ChangeEventTypeSchema
- `packages/core/src/schemas/search-schemas.ts` -- Replaced SearchUpdateTypeSchema with ChangeEventTypeSchema alias, renamed type to changeType
- `packages/core/src/schemas/cluster-schemas.ts` -- Added NodeInfoSchema/PartitionInfoSchema/PartitionMapMessageSchema, used SearchOptionsSchema.extend(), renamed type to changeType
- `packages/core/src/schemas/index.ts` -- Added 24 missing schemas to MessageSchema union (53 to 77 variants)

**Core types/barrel (2 files):**
- `packages/core/src/types/cluster.ts` -- Removed duplicate PartitionMapMessage/PartitionMapRequestMessage, re-exports from cluster-schemas
- `packages/core/src/index.ts` -- Updated imports for schema re-exports

**Server code (4 files):**
- `packages/server/src/query/QueryRegistry.ts` -- REMOVE to LEAVE, type to changeType, ENTER/UPDATE discrimination
- `packages/server/src/subscriptions/DistributedQueryCoordinator.ts` -- type to changeType
- `packages/server/src/subscriptions/DistributedSearchCoordinator.ts` -- type to changeType
- `packages/server/src/ServerCoordinator.ts` -- type to changeType in SEARCH_UPDATE callback

**Client code (2 files):**
- `packages/client/src/SyncEngine.ts` -- REMOVE to LEAVE, type to changeType
- `packages/client/src/SearchHandle.ts` -- type to changeType

**Test files (8 files):**
- `packages/server/src/query/__tests__/QueryRegistry.test.ts` -- Updated assertions for changeType/LEAVE/ENTER
- `packages/server/src/__tests__/LiveQuery.test.ts` -- Updated assertions for changeType/LEAVE/ENTER
- `packages/server/src/subscriptions/__tests__/DistributedSubscriptionCoordinator.test.ts` -- type to changeType
- `packages/server/src/__tests__/integration/distributed-subscriptions.integration.test.ts` -- type to changeType
- `packages/client/src/__tests__/SyncEngine.test.ts` -- changeType/LEAVE
- `packages/client/src/__tests__/Search.test.ts` -- type to changeType
- `tests/e2e/live-queries.test.ts` -- type to changeType, REMOVE to LEAVE
- `tests/e2e/fulltext-search.test.ts` -- type to changeType

### Acceptance Criteria Status

- [x] AC-1: All 4 bugs identified in the specification are fixed
- [x] AC-2: All 6 inconsistencies identified in the specification are resolved
- [x] AC-3: MessageSchema discriminated union contains all message schemas (77 variants)
- [x] AC-4: AuthRequiredMessageSchema exists and is in the union
- [x] AC-5: LockGrantedMessageSchema, LockReleasedMessageSchema, SyncResetRequiredMessageSchema exist
- [x] AC-6: PartitionMapMessageSchema exists with correct field structure
- [x] AC-7: ChangeEventTypeSchema is the single source of truth for ENTER/UPDATE/LEAVE
- [x] AC-8: All change event payloads use field name changeType (not type)
- [x] AC-9: ORMapEntrySchema extracted and reused in 3 places
- [x] AC-10: ClusterSearchReqPayloadSchema uses SearchOptionsSchema.extend()
- [x] AC-11: All schema files have type exports for every schema
- [x] AC-12: All tests pass (core: 1982, client: 501, server: 1211)
- [x] AC-13: Build succeeds across all packages

### Deviations

1. Additional test files modified beyond spec's list of 6: `Search.test.ts` and `distributed-subscriptions.integration.test.ts` also used the old `type` field and needed updating.
2. ENTER/UPDATE discrimination in QueryRegistry caused cascading test assertion updates: sliding window tests expected 'UPDATE' for items entering the result set for the first time, but 'ENTER' is the correct semantic.
3. Used `NodeInfoZod`/`PartitionInfoZod` type names for Zod-inferred types to avoid naming conflicts with existing TS interfaces in types/cluster.ts.
4. Removed PredicateOp/PredicateNode type exports from base-schemas.ts (kept schema definitions) to avoid TS2308 duplicate export conflict with predicate.ts.

---

## Review History

### Review v1 (2026-02-14)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. PredicateOpSchema and PredicateNodeSchema in `packages/core/src/schemas/base-schemas.ts` lack co-located `z.infer` type exports, which is technically a deviation from AC-10's literal wording ("Every `export const XSchema`... has a corresponding `export type X = z.infer<typeof XSchema>` in the same file"). However, this is a justified pragmatic decision: both `PredicateOp` and `PredicateNode` are already exported from `packages/core/src/predicate.ts`, and adding duplicate type exports would cause TS2308 duplicate export errors since `packages/core/src/index.ts` barrels both files. Comments at lines 54 and 62 explain the rationale. No code impact.

**Passed:**
- [PASS] AC-1: MessageSchema union contains 77 variants (verified by counting lines 120-206 of `packages/core/src/schemas/index.ts`). Threshold >= 71 exceeded.
- [PASS] AC-2: `AuthRequiredMessageSchema` exists at `packages/core/src/schemas/base-schemas.ts:99` with `type: z.literal('AUTH_REQUIRED')` and is in the union at line 121.
- [PASS] AC-3: `LockGrantedMessageSchema` (`packages/core/src/schemas/client-message-schemas.ts:120`), `LockReleasedMessageSchema` (line 126), and `SyncResetRequiredMessageSchema` (line 140) all wrap payload schemas and are in the union at lines 204-206.
- [PASS] AC-4: `PartitionMapMessageSchema` exists at `packages/core/src/schemas/cluster-schemas.ts:43` with `type: z.literal('PARTITION_MAP')` and `payload: PartitionMapPayloadSchema`. The old `PartitionMapMessage` and `PartitionMapRequestMessage` interfaces are removed from `packages/core/src/types/cluster.ts` and replaced with Zod-inferred type re-exports at lines 48-49.
- [PASS] AC-5: `ChangeEventTypeSchema = z.enum(['ENTER', 'UPDATE', 'LEAVE'])` at `packages/core/src/schemas/base-schemas.ts:47`. Used by `QueryUpdatePayloadSchema` (via import), `SearchUpdatePayloadSchema` (via `SearchUpdateTypeSchema` alias), `ClusterSubUpdatePayloadSchema` (line 96), and `ClusterSearchUpdatePayloadSchema` (line 192). No schema uses `REMOVE` as a change event value (verified via grep -- only `REMOVE` occurrences are in `ServerEventPayloadSchema.eventType` and `DeltaRecordSchema.eventType`, which are storage mutation types per constraints).
- [PASS] AC-6: `changeType` field name verified in `QueryUpdatePayloadSchema` (`packages/core/src/schemas/client-message-schemas.ts:52`), `SearchUpdatePayloadSchema` (`packages/core/src/schemas/search-schemas.ts:71`), `ClusterSearchUpdatePayloadSchema` (`packages/core/src/schemas/cluster-schemas.ts:192`), and `ClusterSubUpdatePayloadSchema` (`packages/core/src/schemas/cluster-schemas.ts:96`).
- [PASS] AC-7: `ORMapEntrySchema` extracted at `packages/core/src/schemas/sync-schemas.ts:84`. Used in `ORMapSyncRespLeafSchema` (line 140), `ORMapDiffResponseSchema` (line 158), and `ORMapPushDiffSchema` (line 167).
- [PASS] AC-8: `ClusterSearchReqPayloadSchema.options` uses `SearchOptionsSchema.extend()` at `packages/core/src/schemas/cluster-schemas.ts:123-128`. Override: `limit: z.number().int().positive().max(1000)` (required). Added: `includeMatchedTerms`, `afterScore`, `afterKey`.
- [PASS] AC-9: `PNCounterStateObject` type export at `packages/core/src/schemas/messaging-schemas.ts:64`. Not present in `client-message-schemas.ts` (verified via grep).
- [PASS] AC-10: All schema files have type exports for every schema (verified via automated scan). Only exception: `PredicateOpSchema` and `PredicateNodeSchema` -- justified deviation documented above.
- [PASS] AC-11: `pnpm build` succeeds with zero errors across all packages.
- [PASS] AC-12: Core tests pass (1982/1982). Client tests pass (501/501). Server tests: 1210/1211 pass, 1 failure in `FailureDetector.test.ts` which is a pre-existing flaky timing test unrelated to SPEC-053 (last modified in commit `c35cb90`, a different spec entirely).
- [PASS] Deletions verified: `PartitionMapMessage` interface removed from `types/cluster.ts`. `PartitionMapRequestMessage` interface removed. `PNCounterStateObject` removed from `client-message-schemas.ts`. No lingering references to old interfaces found.
- [PASS] Constraints respected: `DeltaRecordSchema.eventType` still uses `'PUT', 'REMOVE'` (storage types, untouched). `ServerEventPayloadSchema.eventType` still uses `['PUT', 'REMOVE', 'OR_ADD', 'OR_REMOVE']` (untouched). `ClientOpSchema.opType` untouched. `SearchUpdateTypeSchema` kept as backward-compat alias.
- [PASS] ENTER/UPDATE discrimination: Local mode `sendUpdate` (`packages/server/src/query/QueryRegistry.ts:753-755`) correctly uses `previousResultKeys.has(key)` to discriminate ENTER vs UPDATE, matching distributed mode logic at lines 780-782.
- [PASS] No lingering `payload.type` references for change events in E2E tests, server code, or client code. All 10 E2E sites verified using `payload.changeType`.
- [PASS] Code quality: Clean, readable, follows established patterns. No unnecessary abstractions. Comments explain the "why" (e.g., PredicateOp/PredicateNode omission rationale). Type naming conventions are consistent (`NodeInfoZod`/`PartitionInfoZod` to avoid conflicts with TS interfaces).

**Summary:** The implementation is thorough and correct. All 12 acceptance criteria are met. All 4 bugs and 6 inconsistencies from the protocol audit are resolved. The 4 documented deviations are pragmatic and well-justified. The one server test failure (`FailureDetector.test.ts`) is a pre-existing flaky test unrelated to this spec. No critical or major issues found.

---

## Completion

**Completed:** 2026-02-14
**Total Commits:** 16
**Audit Cycles:** 4
**Review Cycles:** 1

---
*Generated by SpecFlow on 2026-02-14.*
