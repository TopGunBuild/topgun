---
id: SPEC-144
type: refactor
status: done
priority: P1
complexity: medium
created: 2026-03-24
source: TODO-183
delta: true
---

# Merge ShapeHandle/ShapeManager into QueryHandle/QueryManager

## Context

TopGun's TS client currently has two parallel subscription systems for filtered data:

- **QueryHandle/QueryManager**: Full-featured query system with sort, pagination, ChangeTracker, but no field projection and no Merkle delta reconnect.
- **ShapeHandle/ShapeManager**: Lightweight shape system with field projection (`fields`), Merkle root hash for delta reconnect, but no sort, pagination, or ChangeTracker.

Both use the same `PredicateNode` filter syntax from `@topgunbuild/core`. The server side was already unified in SPEC-142 (wire protocol: `fields` on QUERY_SUB, `merkleRootHash` on QUERY_RESP, QUERY_SYNC_INIT message) and SPEC-143 (server domain: field projection on QUERY_RESP/QUERY_UPDATE, per-query Merkle trees, QUERY_SYNC_INIT handler). This spec completes the client-side merge so `client.query()` becomes the single API for all filtered subscriptions.

This is step 3/4 of the Query+Shape merge initiative. TODO-184 (final cleanup removing Shape wire types) depends on this.

## Goal Analysis

**Goal Statement:** A single `client.query()` API handles all filtered subscription use cases -- with optional field projection, Merkle delta reconnect, sort, pagination, and change tracking.

**Observable Truths:**
1. `client.query('users', { fields: ['name', 'email'] })` returns a `QueryHandle` that receives field-projected results from the server
2. On reconnect, queries with `fields` send QUERY_SYNC_INIT with stored `merkleRootHash` for delta sync instead of full re-fetch
3. `syncEngine.subscribeShape()` still works but is marked `@deprecated` and continues to delegate to ShapeManager unchanged
4. `useQuery` hook accepts `fields` in its filter parameter without any hook-level code changes
5. All existing QueryHandle features (sort, pagination, ChangeTracker, onChanges) continue working unchanged

**Required Artifacts:**
- `packages/client/src/QueryHandle.ts` — extended with `fields` and `merkleRootHash`
- `packages/client/src/sync/QueryManager.ts` — updated `sendQuerySubscription()` and `resubscribeAll()`
- `packages/client/src/SyncEngine.ts` — `subscribeShape()` annotated `@deprecated`, QUERY_RESP handler updated
- `tests/integration-rust/queries.test.ts` — new field projection and Merkle reconnect test cases

**Required Wiring:**
- `QueryFilter.fields` flows from `client.query()` call → `QueryHandle` constructor → `sendQuerySubscription()` → QUERY_SUB wire message
- `merkleRootHash` flows from QUERY_RESP wire message → SyncEngine QUERY_RESP handler → `query.onResult(results, 'server', merkleRootHash)` → `QueryHandle.merkleRootHash` property
- On reconnect, `QueryManager.resubscribeAll()` reads `query.merkleRootHash` and sends QUERY_SYNC_INIT when non-zero and `fields` is set

**Key Links:**
- Wire protocol: SPEC-142 — `fields` on QuerySubPayload, `merkleRootHash` on QueryRespPayload, QUERY_SYNC_INIT message
- Server domain: SPEC-143 — field projection, per-query Merkle trees, QUERY_SYNC_INIT handler
- Planned cleanup (Shape wire type removal): TODO-184

## Delta

### MODIFIED
- `packages/client/src/QueryHandle.ts` -- Add `fields` readonly property, `merkleRootHash` property, store fields from QueryFilter
  - QueryFilter interface: add `fields?: string[]` property
  - Constructor: store `fields` from filter
  - Add `merkleRootHash: number` public property (default 0)
  - `onResult()`: extract and store `merkleRootHash` from server response when present
- `packages/client/src/sync/QueryManager.ts` -- Merge ShapeManager reconnect logic
  - `sendQuerySubscription()`: include `fields` from query filter in QUERY_SUB payload
  - `resubscribeAll()`: for queries with `fields`, send QUERY_SYNC_INIT with stored `merkleRootHash` after QUERY_SUB
- `packages/client/src/SyncEngine.ts` -- Annotate `subscribeShape()` with `@deprecated`; update QUERY_RESP handler to pass `merkleRootHash` to `query.onResult()`
- `tests/integration-rust/queries.test.ts` -- Add test cases for query with `fields` and Merkle reconnect

## Requirements

### R1: Extend QueryFilter with `fields`

**File:** `packages/client/src/QueryHandle.ts`

Add `fields?: string[]` to the `QueryFilter` interface:

```typescript
export interface QueryFilter {
  where?: Record<string, any>;
  predicate?: PredicateNode;
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  cursor?: string;
  fields?: string[];
}
```

### R2: Add `merkleRootHash` and `fields` to QueryHandle

**File:** `packages/client/src/QueryHandle.ts`

- Add public `readonly fields: string[] | undefined` property, populated from `this.filter.fields` in constructor.
- Add public `merkleRootHash: number = 0` property.
- In `onResult()`: accept an optional `merkleRootHash` parameter (or extract from a metadata object). When provided, store it on `this.merkleRootHash`. This is called by SyncEngine when processing QUERY_RESP that contains `merkleRootHash`.

Specifically, update the `onResult` signature to accept merkle hash:

```typescript
public onResult(
  items: { key: string, value: T }[],
  source: QueryResultSource = 'server',
  merkleRootHash?: number
): void {
  // ... existing logic ...
  if (merkleRootHash !== undefined) {
    this.merkleRootHash = merkleRootHash;
  }
}
```

Update the SyncEngine message handler that calls `query.onResult()` to pass through `payload.merkleRootHash` from the QUERY_RESP message.

### R3: Include `fields` in QUERY_SUB message

**File:** `packages/client/src/sync/QueryManager.ts`

Update `sendQuerySubscription()` to include `fields` when present:

```typescript
private sendQuerySubscription(query: QueryHandle<any>): void {
  const filter = query.getFilter();
  this.config.sendMessage({
    type: 'QUERY_SUB',
    payload: {
      queryId: query.id,
      mapName: query.getMapName(),
      query: filter,
      fields: filter.fields,  // Pass fields to server
    }
  });
}
```

### R4: Merkle delta reconnect in QueryManager.resubscribeAll()

**File:** `packages/client/src/sync/QueryManager.ts`

After re-sending QUERY_SUB for each query, check if the query has `fields` set AND a non-zero `merkleRootHash`. If so, send QUERY_SYNC_INIT:

```typescript
public resubscribeAll(): void {
  // ... existing standard query resubscription ...
  for (const query of this.queries.values()) {
    this.sendQuerySubscription(query);

    // Delta reconnect for queries with field projection
    const filter = query.getFilter();
    if (filter.fields && filter.fields.length > 0 && query.merkleRootHash !== 0) {
      this.config.sendMessage({
        type: 'QUERY_SYNC_INIT',
        payload: {
          queryId: query.id,
          rootHash: query.merkleRootHash,
        }
      });
    }
  }
  // ... existing hybrid query resubscription ...
}
```

Note: Merkle delta reconnect is intentionally limited to queries with `fields` because the server only builds per-query Merkle trees for field-projected queries (established in SPEC-143). Non-projected queries perform full re-fetch on reconnect.

### R5: Deprecate SyncEngine.subscribeShape()

**File:** `packages/client/src/SyncEngine.ts`

Mark `subscribeShape()` with `@deprecated` JSDoc tag. The method continues to delegate to `this.shapeManager.subscribeShape()` as before -- no behavioral change. ShapeManager remains functional for the deprecation period.

```typescript
/**
 * @deprecated Use client.query() with { fields } instead. Will be removed in a future version.
 * Subscribe to a shape (partial replication).
 * Delegates to ShapeManager.
 */
public subscribeShape(mapName: string, options?: ShapeSubscribeOptions): ShapeHandle {
  return this.shapeManager.subscribeShape(mapName, options);
}
```

### R6: Pass merkleRootHash from QUERY_RESP to QueryHandle

**File:** `packages/client/src/SyncEngine.ts` (or the message handler file that processes QUERY_RESP)

Locate the handler that processes QUERY_RESP messages and calls `query.onResult()`. The handler destructures the payload; add `merkleRootHash` to the destructure and pass it as the third argument:

```typescript
// In the QUERY_RESP handler:
const { queryId, results, nextCursor, hasMore, cursorStatus, merkleRootHash } = message.payload;
// ...
query.onResult(results, 'server', merkleRootHash);
```

### R7: Integration tests

**File:** `tests/integration-rust/queries.test.ts`

Add at minimum two test cases:

1. **Query with fields projection**: Call `client.query('mapName', { fields: ['name'] })`, write data with multiple fields, verify received results only contain projected fields.
2. **Query reconnect with Merkle delta**: Establish a query with `fields`, receive initial results (merkleRootHash stored), disconnect and reconnect, verify QUERY_SYNC_INIT is sent (server responds with delta only, not full snapshot).

## Acceptance Criteria

1. `QueryFilter` interface includes `fields?: string[]`
2. `QueryHandle` exposes `readonly fields: string[] | undefined` and `merkleRootHash: number`
3. QUERY_SUB messages include `fields` when the query filter specifies field projection
4. On reconnect, queries with `fields` and non-zero `merkleRootHash` send QUERY_SYNC_INIT with `rootHash`
5. `syncEngine.subscribeShape()` is marked `@deprecated` but continues to function
6. `useQuery('map', { fields: ['a'] })` works without any hook code changes
7. Existing query tests pass unchanged
8. At least one integration test exercises `client.query()` with `fields`

## Validation Checklist

- Run `pnpm --filter @topgunbuild/client test` -- all existing and new tests pass
- Run `pnpm --filter @topgunbuild/react test` -- all tests pass (no changes expected)
- Run `pnpm test:integration-rust` -- all 55+ integration tests pass including new field projection test
- Call `client.query('users', { predicate: Predicates.equal('status', 'active'), fields: ['name'] })` -- returns QueryHandle with `fields` set to `['name']`
- TypeScript build `pnpm build` completes with no type errors

## Constraints

- Do NOT remove ShapeHandle, ShapeManager, or Shape wire types (SHAPE_SUBSCRIBE, SHAPE_RESP, etc.) -- that is TODO-184
- Do NOT modify any Rust server code -- server-side merge is already complete (SPEC-142, SPEC-143)
- Do NOT change the `useQuery` hook implementation -- `fields` flows through `QueryFilter` automatically
- Do NOT break existing `subscribeShape()` callers -- deprecation only, no removal
- Keep ShapeManager.resubscribeAll() in the SyncEngine reconnect flow for existing ShapeHandles

## Assumptions

- The QUERY_RESP message handler in SyncEngine (or its message routing module) already has access to the full `QueryRespPayload` including `merkleRootHash` -- it just needs to pass it through
- The server already handles `fields` on QUERY_SUB and returns `merkleRootHash` on QUERY_RESP (confirmed by SPEC-142/143)
- No changes to `@topgunbuild/core` schemas are needed -- SPEC-142 already added `fields` to QuerySubMessage and `merkleRootHash` to QueryRespPayload
- The `useQuery` hook's `QueryFilter` type reference will pick up the `fields` addition automatically since it imports from `@topgunbuild/client`

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Extend QueryFilter with `fields`, add `merkleRootHash` and `fields` properties to QueryHandle (R1, R2) | -- | ~25% |
| G2 | 2 | Update QueryManager: include `fields` in QUERY_SUB, add Merkle reconnect to resubscribeAll (R3, R4) | G1 | ~25% |
| G3 | 2 | Update SyncEngine: deprecate subscribeShape, pass merkleRootHash from QUERY_RESP (R5, R6) | G1 | ~20% |
| G4 | 3 | Integration tests: query with fields, Merkle reconnect (R7) | G2, G3 | ~30% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-24)
**Status:** NEEDS_REVISION

**Context Estimate:** ~45% total

Delta validation: 5/7 entries valid

**Critical:**
1. **Observable Truth 3 contradicts R5:** Truth 3 states `subscribeShape()` "delegates internally to QueryManager," but R5 explicitly keeps it delegating to `this.shapeManager.subscribeShape()` with no behavioral change. The truth describes a delegation rewrite that is not specified in the requirements. Fix Truth 3 to say: "still works but is marked `@deprecated` and continues to delegate to ShapeManager."
2. **R6 code snippet does not match actual code:** R6 shows `payload.results.map(r => ({ key: r.key, value: r.value }))` but the actual `handleQueryResp` at SyncEngine line 830 passes `results` directly (already destructured from `message.payload`). The snippet should be: `query.onResult(results, 'server', message.payload.merkleRootHash)` -- or since `merkleRootHash` is not in the destructure, add it: `const { queryId, results, nextCursor, hasMore, cursorStatus, merkleRootHash } = message.payload;` then `query.onResult(results, 'server', merkleRootHash);`.
3. **Delta MODIFIED lists files with no changes:** `packages/client/src/TopGunClient.ts` and `packages/react/src/hooks/useQuery.ts` are listed as MODIFIED but both explicitly say "No changes needed." Remove these from the Delta section -- they create false expectations for the implementer.

**Recommendations:**
4. **Delta SyncEngine description inconsistency:** The Delta bullet for SyncEngine.ts says `subscribeShape()` should "create a QueryHandle via QueryManager with fields/limit/filter, return a thin ShapeHandle wrapper." But R5 says the method keeps its existing delegation to ShapeManager unchanged. Align the Delta description with R5 (deprecation annotation only).
5. **Goal Analysis missing subsections:** Goal Analysis is present but missing Required Artifacts, Required Wiring, and Key Links subsections. These are recommended for medium-complexity specs to ensure complete traceability.
6. [Strategic] Consider whether Merkle delta reconnect should apply to ALL queries (not just those with `fields`). The current design ties Merkle reconnect to field projection, but non-projected queries would also benefit from delta sync. This may be intentional (server only builds Merkle trees for projected queries per SPEC-143), but worth confirming.

### Response v1 (2026-03-24)
**Applied:** all critical issues and all recommendations

**Changes:**
1. [✓] Observable Truth 3 — reworded to "continues to delegate to ShapeManager unchanged" to match R5
2. [✓] R6 code snippet — replaced `payload.results.map(...)` with the correct destructure pattern: `const { queryId, results, nextCursor, hasMore, cursorStatus, merkleRootHash } = message.payload;` then `query.onResult(results, 'server', merkleRootHash);`
3. [✓] Delta MODIFIED — removed `TopGunClient.ts` and `useQuery.ts` entries from the MODIFIED list
4. [✓] Delta SyncEngine description — revised bullet to describe deprecation annotation and QUERY_RESP handler update only, removing the "create a QueryHandle via QueryManager / thin ShapeHandle wrapper" language
5. [✓] Goal Analysis subsections — added Required Artifacts, Required Wiring, and Key Links subsections
6. [✓] Strategic note on Merkle reconnect scope — added a Note paragraph in R4 confirming the intentional tie to `fields` because SPEC-143 only builds per-query Merkle trees for field-projected queries

### Audit v2 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~45% total

Delta validation: 5/5 entries valid

**Dimensions:**
- Clarity: All 7 requirements specify exact file, method, and code changes. No vague terms.
- Completeness: All modified files listed in Delta with detailed change descriptions. Wire protocol types confirmed in `@topgunbuild/core` (SPEC-142 already added `fields` to QuerySubMessage, `merkleRootHash` to QueryRespPayload, QUERY_SYNC_INIT message).
- Testability: All 8 acceptance criteria are concrete and verifiable.
- Scope: Clear boundaries with 5 explicit constraints on what NOT to change.
- Feasibility: Verified against actual source code -- all assumptions confirmed (QueryFilter at line 6, onResult at line 111, handleQueryResp at line 826, sendQuerySubscription at line 90, resubscribeAll at line 313, subscribeShape at line 654).
- Architecture fit: Extends existing QueryFilter/QueryHandle/QueryManager patterns naturally.
- Non-duplication: This spec explicitly reduces duplication by merging two parallel systems.
- Cognitive load: Minimal -- adds optional properties to existing types, no new abstractions.
- Strategic fit: Aligned with project goals. Step 3/4 of a well-sequenced merge initiative.
- Project compliance: All TypeScript packages. Language Profile notes explicitly exempt TS from Rust-specific constraints (file limit, trait-first).

**Goal-Backward Validation:**
- Truth 1 (field projection) covered by artifacts: QueryHandle.ts, QueryManager.ts, queries.test.ts
- Truth 2 (Merkle reconnect) covered by artifacts: QueryHandle.ts, QueryManager.ts, queries.test.ts
- Truth 3 (deprecated subscribeShape) covered by artifact: SyncEngine.ts
- Truth 4 (useQuery with fields) covered by: no code change needed, type flows automatically
- Truth 5 (existing features unchanged) covered by: constraint "do NOT break" + validation checklist
- All wiring paths have matching requirements (R1-R6)
- Key links identified with external dependency references

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | QUERY_RESP handler has access to full payload including merkleRootHash | R6 would need different approach to extract the field |
| A2 | Server handles fields on QUERY_SUB (SPEC-142/143) | Field projection would not work; but verified in core schemas |
| A3 | No core schema changes needed | Would expand scope to core package; but verified -- schemas already include fields/merkleRootHash |
| A4 | useQuery imports QueryFilter from client package | Hook would need separate type update; standard monorepo pattern confirms this |

All assumptions verified against source code. Impact is low even if wrong.

**Project compliance:** Honors PROJECT.md decisions. No Rust changes, no new dependencies, follows existing TS conventions.
**Strategic fit:** Aligned with project goals -- completes client-side merge of duplicate subscription systems.

**Recommendations:**
1. Delta entry for `tests/integration-rust/shape.test.ts` describes changes as "optionally duplicate key tests" which is vague for an implementer. Consider either making it concrete (specify which tests to duplicate) or removing it from the Delta since no Requirement covers shape.test.ts changes.

**Comment:** Well-structured spec with clear requirements backed by accurate code snippets. All v1 critical issues were properly addressed. The spec is ready for implementation.

### Response v2 (2026-03-24)
**Applied:** audit v2 recommendation

**Changes:**
1. [✓] Removed vague `tests/integration-rust/shape.test.ts` entry from Delta section — no Requirement covers shape.test.ts changes, and "optionally duplicate key tests" was too vague for an implementer

## Execution Summary

**Executed:** 2026-03-24
**Mode:** orchestrated
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |
| 3 | G4 | complete |

### Files Modified

- `packages/client/src/QueryHandle.ts` — added `fields?: string[]` to QueryFilter, added `readonly fields` and `merkleRootHash` properties to QueryHandle, updated `onResult()` to accept optional `merkleRootHash`
- `packages/client/src/sync/QueryManager.ts` — updated `sendQuerySubscription()` to include `fields` in QUERY_SUB payload, updated `resubscribeAll()` to send QUERY_SYNC_INIT for field-projected queries with stored Merkle hash
- `packages/client/src/SyncEngine.ts` — marked `subscribeShape()` `@deprecated`, updated `handleQueryResp()` to extract and pass `merkleRootHash` to `query.onResult()`
- `packages/client/src/__tests__/SyncEngine.test.ts` — updated assertion to expect optional third argument in `onResult` call
- `tests/integration-rust/queries.test.ts` — added field projection test and Merkle delta reconnect test

### Acceptance Criteria Status

- [x] `QueryFilter` interface includes `fields?: string[]`
- [x] `QueryHandle` exposes `readonly fields: string[] | undefined` and `merkleRootHash: number`
- [x] QUERY_SUB messages include `fields` when the query filter specifies field projection
- [x] On reconnect, queries with `fields` and non-zero `merkleRootHash` send QUERY_SYNC_INIT with `rootHash`
- [x] `syncEngine.subscribeShape()` is marked `@deprecated` but continues to function
- [x] `useQuery('map', { fields: ['a'] })` works without any hook code changes (type flows automatically)
- [x] Existing query tests pass unchanged (461 client tests, 63 integration tests pass)
- [x] At least one integration test exercises `client.query()` with `fields`

### Deviations

None. All requirements implemented as specified.

---

## Review History

### Review v1 (2026-03-24)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `QueryFilter` interface includes `fields?: string[]` at line 14 of QueryHandle.ts
- [✓] AC2: `QueryHandle` exposes `readonly fields: string[] | undefined` (line 54) and `merkleRootHash: number = 0` (line 57)
- [✓] AC3: `sendQuerySubscription()` in QueryManager.ts includes `fields: filter.fields` in QUERY_SUB payload (line 99)
- [✓] AC4: `resubscribeAll()` sends QUERY_SYNC_INIT for queries with `fields` and non-zero `merkleRootHash` (lines 326-334 of QueryManager.ts)
- [✓] AC5: `subscribeShape()` carries `@deprecated` JSDoc at SyncEngine.ts line 651 and continues delegating to `this.shapeManager.subscribeShape()`
- [✓] AC6: `QueryFilter.fields` is exported from client package index and flows to `useQuery` automatically — no hook changes needed
- [✓] AC7: 461 client tests and 182 react tests pass unchanged; build clean
- [✓] AC8: Two integration test cases added to queries.test.ts (lines 1113-1278) — field projection test and Merkle delta reconnect test
- [✓] R6 wiring: `handleQueryResp` in SyncEngine.ts destructures `merkleRootHash` from payload and passes it as third argument to `query.onResult()` (lines 828, 831)
- [✓] `onResult()` updated signature accepts optional third `merkleRootHash?: number` and stores it (lines 120, 146-148)
- [✓] SyncEngine.test.ts updated to expect `undefined` as third argument when no `merkleRootHash` in payload (line 403)
- [✓] ShapeHandle, ShapeManager, and Shape wire types untouched — constraint respected
- [✓] No Rust server changes — constraint respected
- [✓] Build completes cleanly (CJS + ESM + DTS)

**Summary:** All 8 acceptance criteria met. Implementation follows specification exactly with no deviations. All tests pass, build is clean, and the constraint boundaries (no Shape removal, no Rust changes, no useQuery changes) are honored throughout.

---

## Completion

**Completed:** 2026-03-24
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Unified the client-side filtered subscription API by merging ShapeHandle/ShapeManager capabilities (field projection, Merkle delta reconnect) into QueryHandle/QueryManager. `client.query()` is now the single API for all filtered subscriptions.

### Key Files

- `packages/client/src/QueryHandle.ts` — Extended QueryFilter and QueryHandle with fields/merkleRootHash
- `packages/client/src/sync/QueryManager.ts` — QUERY_SUB includes fields, resubscribeAll sends QUERY_SYNC_INIT for delta reconnect
- `packages/client/src/SyncEngine.ts` — subscribeShape() deprecated, QUERY_RESP passes merkleRootHash through

### Changes Applied

**Modified:**
- `packages/client/src/QueryHandle.ts` — Added `fields?: string[]` to QueryFilter, `readonly fields` and `merkleRootHash` properties to QueryHandle, updated `onResult()` signature
- `packages/client/src/sync/QueryManager.ts` — `sendQuerySubscription()` includes fields in QUERY_SUB, `resubscribeAll()` sends QUERY_SYNC_INIT for field-projected queries
- `packages/client/src/SyncEngine.ts` — `@deprecated` on subscribeShape(), handleQueryResp passes merkleRootHash to onResult()
- `packages/client/src/__tests__/SyncEngine.test.ts` — Updated assertion for onResult third argument
- `tests/integration-rust/queries.test.ts` — Added field projection and Merkle delta reconnect test cases

### Patterns Established

None — followed existing patterns.

### Spec Deviations

None — implemented as specified.
