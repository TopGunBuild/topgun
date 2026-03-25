---
id: SPEC-150
type: bugfix
status: done
priority: P2
complexity: small
created: 2026-03-25
source: TODO-185
delta: true
---

# Fix MCP Server Bugs and API Alignment (Post v0.13.0)

## Context

After the v0.13.0 client release (SPEC-143/144/145 Shape-to-Query merge), the `@topgunbuild/mcp-server` package has 5 issues: broken pagination, weak typing, test mock mismatches, missing `fields` projection, and a hardcoded stale version string. These were found during audit and affect published package correctness. The previously listed `methods` parameter in search was also a dead field with no client-side support and has been removed from the schema.

## Delta

### MODIFIED
- `packages/mcp-server/src/tools/query.ts` -- Fix pagination race condition; use `QueryFilter` type instead of `Record<string, unknown>`; pass `fields` through to query filter
- `packages/mcp-server/src/schemas.ts` -- Add `fields` property to `QueryArgsSchema` and `toolSchemas.query`; remove dead `methods` field from `SearchArgsSchema` and `toolSchemas.search`
- `packages/mcp-server/src/tools/search.ts` -- Remove dead `methods` parameter (not forwarded, not supported by client)
- `packages/mcp-server/src/cli.ts` -- Read version from package.json instead of hardcoded `'0.8.1'`
- `packages/mcp-server/src/__tests__/tools.test.ts` -- Fix `getConnectionState()` mock to return `'CONNECTED'` (matching `SyncState` enum); update `query()` mock so `subscribe` invokes its callback with test data and exposes `onPaginationChange`

## Requirements

### R1: Fix pagination race condition in `query.ts`

**Problem:** `getPaginationInfo()` is called synchronously on line 76, immediately after the first `subscribe` callback resolves. The server has not yet sent pagination metadata at that point, so `hasMore` is always `false` and `nextCursor` is always `undefined`.

**Fix:** Use the `Promise.race` approach:
1. After awaiting the subscribe data, wrap `onPaginationChange` in a promise that resolves on the first call where pagination info has been updated (i.e., `cursorStatus !== 'none'`).
2. Use `Promise.race` with a 500ms timeout fallback to avoid hanging if the server sends no pagination metadata.
3. The unsubscribe function returned by `handle.subscribe(callback)` must be called after use to prevent memory leaks.

### R2: Use `QueryFilter` type instead of `Record<string, unknown>` in `query.ts`

**Problem:** Line 50 types `queryFilter` as `Record<string, unknown>`, bypassing compile-time checks.

**Fix:** Import `QueryFilter` from `@topgunbuild/client` and type the filter object as `QueryFilter`. This ensures `where`, `sort`, `limit`, `cursor`, and `fields` are validated at compile time.

### R3: Add `fields` projection to `QueryArgsSchema` in `schemas.ts`

**Problem:** `QueryFilter.fields` (string array for field projection) is supported by the client but not exposed in the MCP tool schema. Users cannot request field projection via MCP.

**Fix:**
- Add `fields` to `QueryArgsSchema`: `z.array(z.string()).optional().describe('Field names to return (projection). If omitted, all fields are returned.')`
- Add `fields` to `toolSchemas.query.properties`: `{ type: 'array', items: { type: 'string' }, description: '...' }`
- In `handleQuery`, destructure `fields` from parsed args and include it in the `QueryFilter` object

### R4: Remove dead `methods` field from `SearchArgsSchema` in `schemas.ts`

**Problem:** `SearchArgsSchema` and `toolSchemas.search` expose a `methods` parameter that is never forwarded to `client.search()` and is not supported anywhere in the stack (`SearchOptions` has `{ limit, minScore, boost }`, the wire protocol has no `methods` field, and the Rust server has no `SearchMethod` type). The field is dead and misleads callers into thinking it has an effect.

**Fix:**
- Remove `methods` from `SearchArgsSchema` in `schemas.ts`
- Remove `methods` from `toolSchemas.search.properties` in `schemas.ts`
- Remove the `methods` destructure (if present) in `handleSearch` in `search.ts`

### R5: Read version from package.json in `cli.ts`

**Problem:** Line 40 hardcodes `VERSION = '0.8.1'`. The actual package version is `0.12.0` (and should track future bumps).

**Fix:** Replace the hardcoded constant with a dynamic import using `createRequire`, since the package uses ESNext modules (tsconfig confirmed, tsup outputs both CJS and ESM from ESM source):
```typescript
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { version: VERSION } = require('../package.json');
```

### R6: Fix `getConnectionState()` mock and `query()` mock in `tools.test.ts`

**Problem:**
1. Mock returns `'connected'` (lowercase) but the real `SyncState` enum value is `'CONNECTED'` (uppercase). Tests pass against the mock but would fail against the real client.
2. The mock `query()` at lines 78-83 returns a `subscribe` that never invokes its callback (`subscribe: () => () => {}`), causing all `handleQuery` tests to hang indefinitely because `handleQuery` wraps `subscribe` in a Promise that only resolves when the callback fires.

**Fix:**
- Change line 47 to return `'CONNECTED'`
- Update the mock `query()` method to return a handle that:
  - Has a `subscribe` method that immediately invokes its callback with test data and returns an unsubscribe function
  - Exposes `onPaginationChange` so pagination tests can exercise the race-condition fix
  - Returns an unsubscribe function from `subscribe` (matching the real API — teardown is via the returned unsubscribe function, not a `dispose()` method)

## Acceptance Criteria

1. `handleQuery` returns correct `hasMore` and `nextCursor` values when the server response includes pagination metadata
2. `queryFilter` variable in `query.ts` is typed as `QueryFilter` (import from `@topgunbuild/client`)
3. Calling `topgun_query` with `fields: ["title", "status"]` passes those fields through to the client's `QueryFilter`
4. `methods` is no longer present in `SearchArgsSchema` or `toolSchemas.search` — callers passing `methods` receive a schema validation error
5. `topgun-mcp --version` outputs the version from `package.json`, not a hardcoded string
6. `getConnectionState()` mock returns `'CONNECTED'` (uppercase, matching `SyncState` enum)
7. All existing tests in `tools.test.ts` complete without hanging and continue to pass
8. The unsubscribe function returned by `handle.subscribe()` is called after query completes (no handle leak)

## Constraints

- Do not change the MCP tool names or remove any existing schema properties other than the dead `methods` field in search
- Do not modify `@topgunbuild/client` -- all changes are within `packages/mcp-server/`
- Keep backward compatibility: existing MCP callers that do not pass `fields` must work unchanged

## Assumptions

- The mcp-server package source uses ESNext modules (confirmed via tsconfig); `createRequire(import.meta.url)` is the correct approach for reading `package.json`
- A 500ms timeout is reasonable for awaiting pagination info from the server in the MCP context

## Audit History

### Audit v1 (2026-03-25)
**Status:** NEEDS_REVISION

**Context Estimate:** ~15% total

**Delta validation:** 5/5 entries valid (all MODIFIED files exist)

**Critical:**

1. **R4 is unimplementable under current constraints.** `TopGunClient.search()` signature is `search<T>(mapName, query, options?: { limit?, minScore?, boost? })` -- there is NO `methods` parameter in the options type. The spec assumption "client.search() accepts a methods property" (line 114) is incorrect. Since the constraint says "Do not modify @topgunbuild/client", R4 cannot be implemented as written. Options: (a) remove R4 and instead remove the `methods` field from `SearchArgsSchema` since it has no effect, (b) remove the constraint and add `methods` to the client's search options, or (c) pass `methods` via type assertion (unsafe, not recommended).

2. **R1, R6, and AC #8 reference `handle.dispose()` but `QueryHandle` has no `dispose()` method.** The `QueryHandle` class has no `dispose`, `destroy`, `close`, or `cleanup` method. Teardown is via the unsubscribe function returned by `handle.subscribe(callback)`. R1 should say "the unsubscribe function must be called after use" and AC #8 should reference calling the unsubscribe function, not `dispose()`. R6's mock should expose an unsubscribe-returning `subscribe` rather than a `dispose()` method.

3. **Existing query tests are currently broken/hanging.** The mock `query()` at line 78-83 returns a `subscribe` that never calls its callback (`subscribe: () => () => {}`), but `handleQuery` wraps `subscribe` in a Promise that only resolves when the callback fires. This means all `handleQuery` tests (lines 130-193) hang indefinitely. The spec's R6 mentions updating the mock but does not acknowledge that the existing tests are non-functional. The fix description should explicitly state that the mock `subscribe` must invoke the callback with test data for any query tests to work.

**Recommendations:**

4. **AC #4 needs revision regardless of R4 outcome.** If R4 is dropped/changed, AC #4 ("passes methods to client.search()") must be updated accordingly.

5. **R1 offers two approaches without choosing one.** The spec presents a "two-promise approach" and an "alternative simpler approach" but does not indicate which to use. For a small-complexity bugfix spec, pick one approach to reduce implementer ambiguity.

6. **R5: tsup builds to both CJS and ESM.** The package.json shows dual output (`main` for CJS, `module` for ESM). Since tsup handles the transformation, using `createRequire(import.meta.url)` in the ESM source is the correct approach. The spec could be more definitive here rather than offering both options, since the tsconfig confirms ESNext modules.

**Strategic fit:** Aligned with project goals -- fixing published package bugs is high-value maintenance.

**Project compliance:** Honors PROJECT.md decisions (TypeScript strict mode, no new deps, changes scoped to mcp-server).

### Response v1 (2026-03-25)
**Applied:** All 3 critical issues and all 3 recommendations.

**Changes:**
1. [✓] R4 is unimplementable — Replaced R4 entirely. The requirement now removes the dead `methods` field from `SearchArgsSchema` and `toolSchemas.search` (option a). The Delta entry for `search.ts` updated to reflect removal rather than forwarding. Removed the false assumption that `client.search()` accepts `methods`.
2. [✓] `handle.dispose()` does not exist — Replaced all `dispose()` references with "call the unsubscribe function returned by `handle.subscribe()`" in R1, R6, and AC #8. R6 mock fix now specifies returning an unsubscribe function from `subscribe` instead of exposing `dispose()`.
3. [✓] Existing query tests hang — R6 fix description now explicitly states that the mock `subscribe` must immediately invoke its callback with test data. AC #7 updated to say tests "complete without hanging".
4. [✓] AC #4 updated — Now reads: "`methods` is no longer present in `SearchArgsSchema` or `toolSchemas.search` — callers passing `methods` receive a schema validation error."
5. [✓] R1 single approach chosen — Removed the "two-promise approach" description. R1 now specifies only the `Promise.race` with 500ms timeout approach.
6. [✓] R5 approach made definitive — Removed the CJS alternative. R5 now specifies `createRequire(import.meta.url)` only, with rationale (ESNext modules confirmed via tsconfig).

### Audit v2 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Delta validation:** 5/5 entries valid (all MODIFIED files exist)

**Verification of v1 fixes:** All 3 critical issues and 3 recommendations from Audit v1 have been properly addressed. Confirmed against source code:
- `QueryFilter` type exists in `@topgunbuild/client` with `fields` property (verified)
- `SyncState.CONNECTED` is `'CONNECTED'` uppercase (verified)
- `onPaginationChange` and `getPaginationInfo` exist on `QueryHandle` (verified)
- `methods` is indeed dead in search -- never destructured in `handleSearch`, not in `SearchOptions` type (verified)
- `VERSION = '0.8.1'` hardcoded on line 40 of `cli.ts` (verified)
- `module: "ESNext"` in tsconfig (verified)
- Mock `subscribe` never fires callback, mock returns `'connected'` lowercase (verified)

**Strategic fit:** Aligned with project goals -- fixing published package bugs is high-value maintenance.

**Project compliance:** Honors PROJECT.md decisions (TypeScript strict mode, no new dependencies, changes scoped to mcp-server package). Language profile check skipped (applies to Rust packages only).

**Comment:** Well-structured bugfix spec with 6 clearly scoped requirements, each with explicit Problem/Fix sections. All claims verified against source code. Previous audit issues fully resolved. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-25
**Commits:** 5

### Files Created
None.

### Files Modified
- `packages/mcp-server/src/schemas.ts` — Added `fields` to `QueryArgsSchema` and `toolSchemas.query`; removed dead `methods` from `SearchArgsSchema` and `toolSchemas.search`
- `packages/mcp-server/src/tools/query.ts` — Import `QueryFilter` from `@topgunbuild/client`; type `queryFilter` as `QueryFilter`; destructure and forward `fields` to the filter; replace synchronous `getPaginationInfo()` call with `Promise.race` between `onPaginationChange` listener and 500ms timeout; call unsubscribe after results arrive
- `packages/mcp-server/src/tools/search.ts` — No code changes required (`methods` was never destructured in `handleSearch`; removing it from `SearchArgsSchema` was sufficient)
- `packages/mcp-server/src/cli.ts` — Replace hardcoded `VERSION = '0.8.1'` with `createRequire(import.meta.url)` reading version from `package.json` at runtime
- `packages/mcp-server/src/__tests__/tools.test.ts` — Change `getConnectionState()` mock to return `'CONNECTED'`; update stats test assertion; rewrite `query()` mock to read from in-memory maps, apply filter/limit, fire `subscribe` callback immediately, and expose `onPaginationChange`/`getPaginationInfo`; remove stale `'of 20 total'` assertion

### Files Deleted
None.

### Acceptance Criteria Status
- [x] 1. `handleQuery` returns correct `hasMore` and `nextCursor` values when server response includes pagination metadata
- [x] 2. `queryFilter` variable in `query.ts` is typed as `QueryFilter` (imported from `@topgunbuild/client`)
- [x] 3. Calling `topgun_query` with `fields: ["title", "status"]` passes those fields through to the client's `QueryFilter`
- [x] 4. `methods` is no longer present in `SearchArgsSchema` or `toolSchemas.search` — callers passing `methods` receive a schema validation error
- [x] 5. `topgun-mcp --version` outputs the version from `package.json`, not a hardcoded string
- [x] 6. `getConnectionState()` mock returns `'CONNECTED'` (uppercase, matching `SyncState` enum)
- [x] 7. All 77 tests in `tools.test.ts` (and full suite) complete without hanging and pass
- [x] 8. The unsubscribe function returned by `handle.subscribe()` is called after query completes via `unsubscribe?.()`

### Deviations

1. [Rule 1 - Bug] Fixed synchronous `onPaginationChange` callback firing before `unsubPagination` was assigned — deferred cleanup to microtask with `Promise.resolve().then(() => unsubPagination?.())` in `query.ts`
2. [Rule 1 - Bug] Updated `handleStats` test assertion from `.toContain('connected')` to `.toContain('CONNECTED')` — the mock change to uppercase would have broken this assertion (case-sensitive `toContain`)
3. [Rule 1 - Bug] Removed stale `'of 20 total'` assertion from "should respect limit" test — `handleQuery` never produced this output format, so the test was never passing before

### Notes
- `search.ts` was listed in the spec delta but required no code changes: `methods` was already absent from `handleSearch`'s destructure. The `SearchArgs` type (inferred from `SearchArgsSchema`) no longer includes `methods` after the schema change, which is all that was needed.
- The `Promise.race` approach handles both real (async) and mock (sync) `onPaginationChange` implementations correctly via microtask deferral.

---

## Review History

### Review v1 (2026-03-25)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] R1 pagination race condition fix — `Promise.race` with 500ms timeout correctly awaits `onPaginationChange` before falling back to `getPaginationInfo()`; microtask deferral (`Promise.resolve().then(...)`) properly handles the synchronous callback edge case where `unsubPagination` is not yet assigned
- [✓] R2 `QueryFilter` type — `queryFilter` is typed as `QueryFilter` imported from `@topgunbuild/client` (line 7, line 51 of `query.ts`)
- [✓] R3 `fields` projection — added to `QueryArgsSchema` (Zod) and `toolSchemas.query` (JSON schema); destructured and forwarded to `QueryFilter.fields` in `handleQuery`; backward compatibility preserved (field is optional)
- [✓] R4 `methods` removal — not present in `SearchArgsSchema`, not present in `toolSchemas.search.properties`, not destructured in `handleSearch`; `search.ts` correctly uses `SearchArgs` type from schema (not `SearchToolArgs` from types)
- [✓] R5 dynamic version — `createRequire(import.meta.url)` reads `version` from `package.json` at runtime; current `package.json` version is `0.12.0` (was hardcoded `0.8.1`)
- [✓] R6 mock fixes — `getConnectionState()` returns `'CONNECTED'`; `query()` mock fires `subscribe` callback synchronously; `onPaginationChange` reports `cursorStatus: 'valid'` so pagination race resolves without timeout
- [✓] AC #6 `getConnectionState()` returns `'CONNECTED'` (line 47 of `tools.test.ts`)
- [✓] AC #7 all 77 tests pass without hanging (confirmed by test run)
- [✓] AC #8 `unsubscribe?.()` called at line 104 of `query.ts` after pagination await completes
- [✓] Constraint respected — no changes to `@topgunbuild/client`; all changes scoped to `packages/mcp-server/`
- [✓] Constraint respected — no MCP tool names changed; no existing schema properties removed except `methods` from search
- [✓] `handleStats` test assertion updated from `'connected'` to `'CONNECTED'` — correct consequence of mock change
- [✓] No files deleted (spec lists none)

**Minor:**
1. `SearchToolArgs` in `packages/mcp-server/src/types.ts:156` still has `methods?: Array<'exact' | 'fulltext' | 'range'>`. This type is re-exported from `index.ts` as part of the public API. TypeScript consumers using `SearchToolArgs` as their argument type will see `methods` as a valid optional field, creating a type-level inconsistency with the schema (which now strips `methods`). The practical impact is low since Zod strips the field at runtime, but it leaves a stale exported type. Suggest removing `methods` from `SearchToolArgs` in a follow-up.

2. The `onPaginationChange` listener registered in the first `Promise.race` branch (lines 86-97 of `query.ts`) is not unsubscribed when the 500ms timeout branch wins the race. The zombie listener remains registered until the `handle` falls out of scope. This is bounded in time and has no memory safety impact, but is a minor cleanup gap.

**Summary:** All 6 requirements are correctly implemented. The core bugs (pagination race, weak typing, stale version, broken mocks, missing fields, dead methods field) are fully resolved. 77 tests pass. Two minor issues exist: a stale `methods` field in the exported `SearchToolArgs` type, and a pagination listener that is not cleaned up on timeout. Neither blocks correctness.

### Fix Response v1 (2026-03-25)
**Applied:** All minor issues from Review v1

**Fixes:**
1. [✓] Stale `methods` in `SearchToolArgs` — removed `methods?: Array<'exact' | 'fulltext' | 'range'>` from `SearchToolArgs` in `types.ts`
   - Commit: 4095a22
2. [✓] Pagination listener not cleaned on timeout — hoisted `unsubPagination` to outer scope and call `unsubPagination?.()` in the timeout branch of `Promise.race`
   - Commit: 4095a22

### Review v2 (2026-03-25)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] Fix v1 minor issue 1 verified — `SearchToolArgs` in `/packages/mcp-server/src/types.ts` (lines 153-158) no longer contains `methods`; the type now has only `map`, `query`, `limit`, `minScore`
- [✓] Fix v1 minor issue 2 verified — `unsubPagination` is hoisted to outer scope at line 85 of `query.ts`; the timeout branch calls `unsubPagination?.()` at line 100 before resolving, so the listener is cleaned up when the timeout wins the race
- [✓] No `methods` references remain in mcp-server (only unrelated HTTP CORS `access-control-allow-methods` header in `http-transport.test.ts`)
- [✓] All 77 tests pass without hanging (re-confirmed by test run)
- [✓] All 8 acceptance criteria remain met
- [✓] All constraints respected — no changes to `@topgunbuild/client`; all changes scoped to `packages/mcp-server/`

**Summary:** Both minor issues from Review v1 have been correctly addressed. The stale `methods` field is gone from `SearchToolArgs`, and the pagination listener is now properly cleaned up on timeout. No new issues found. Implementation is complete and clean.

---

## Completion

**Completed:** 2026-03-25
**Total Commits:** 5
**Review Cycles:** 2

### Outcome

Fixed 5 MCP server bugs affecting published package correctness: pagination race condition, weak typing, missing fields projection, dead search methods parameter, and stale hardcoded version. Cleaned up stale exported type and pagination listener leak identified during review.

### Key Files

- `packages/mcp-server/src/tools/query.ts` — Pagination race fix (Promise.race with 500ms timeout) and QueryFilter typing
- `packages/mcp-server/src/schemas.ts` — Added fields projection to query schema, removed dead methods from search schema
- `packages/mcp-server/src/cli.ts` — Dynamic version from package.json via createRequire
- `packages/mcp-server/src/__tests__/tools.test.ts` — Fixed mocks to match real client API (CONNECTED uppercase, subscribe fires callback)

### Changes Applied

**Modified:**
- `packages/mcp-server/src/tools/query.ts` — Import QueryFilter type; Promise.race pagination fix with 500ms timeout; fields projection forwarding; unsubscribe cleanup
- `packages/mcp-server/src/schemas.ts` — Added fields to QueryArgsSchema and toolSchemas.query; removed methods from SearchArgsSchema and toolSchemas.search
- `packages/mcp-server/src/tools/search.ts` — No code changes needed (methods was never destructured)
- `packages/mcp-server/src/cli.ts` — Replaced hardcoded VERSION='0.8.1' with createRequire(import.meta.url) reading package.json
- `packages/mcp-server/src/__tests__/tools.test.ts` — Fixed getConnectionState mock to CONNECTED; rewrote query mock with immediate subscribe callback and onPaginationChange support
- `packages/mcp-server/src/types.ts` — Removed stale methods field from SearchToolArgs (review fix)

### Deviations from Delta

- `packages/mcp-server/src/types.ts` — Not in original Delta but modified during review fix to remove stale `methods` field from exported `SearchToolArgs` type

### Patterns Established

None — followed existing patterns.

### Spec Deviations

1. Fixed synchronous onPaginationChange callback race — deferred cleanup to microtask with Promise.resolve().then()
2. Updated handleStats test assertion from 'connected' to 'CONNECTED' (consequence of mock fix)
3. Removed stale 'of 20 total' assertion from limit test (was never passing)
