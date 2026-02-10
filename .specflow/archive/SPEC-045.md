---
id: SPEC-045
type: bugfix
status: done
priority: P2
complexity: medium
created: 2026-02-09
---

# Fix ProcessorSandbox Test Hang and Update Documentation Server Instantiation

## Context

Two unrelated P2 issues are grouped in this specification:

1. **ProcessorSandbox hang:** The test file `packages/server/src/__tests__/ProcessorSandbox.test.ts` hangs the entire Jest process, even with `--forceExit`. The root cause is in the fallback VM path (`executeInFallback`) of `ProcessorSandbox.ts`. The "timeout on infinite loop" test (line 251) runs `while(true) {}` inside user-provided processor code. In the fallback path, the code executes synchronously via `new Function()`, but the timeout mechanism uses `Promise.race` with a `setTimeout`. Since the synchronous infinite loop blocks the Node.js event loop, the timeout promise's callback never fires — the process hangs forever. The `isolated-vm` path handles this correctly because it runs code in a separate V8 isolate with a native timeout, but isolated-vm is not available in the test environment.

2. **Documentation outdated:** Twelve documentation files still reference `new ServerCoordinator({...})` for server creation. The correct public API is `ServerFactory.create({...})` since the modular refactoring (SPEC-011 series). The `ServerCoordinator` constructor now requires a second `dependencies` argument that only `ServerFactory` assembles, so the documented pattern would not even compile.

## Task

### Part 1: Fix ProcessorSandbox fallback VM timeout

Replace the `Promise.race` timeout mechanism in `executeInFallback` with a mechanism that can actually interrupt synchronous code execution. Two viable approaches:

**Approach A (Preferred): Use Node.js `vm` module with `timeout` option.**
Replace `new Function()` with `vm.runInNewContext()` (or `vm.Script` + `script.runInNewContext()`), passing the `timeout` option. Node.js `vm.runInNewContext` supports a `timeout` parameter that uses V8's `TerminateExecution` to interrupt synchronous infinite loops — unlike `Promise.race` which requires event loop ticks.

**Approach B: Use `worker_threads` with a timeout.**
Spawn a Worker thread, run the code there, and kill the thread on timeout. This is heavier and less appropriate for a "fallback" mode.

After the fix, remove the `Promise.race`/`setTimeout` timeout pattern from `executeInFallback`.

### Part 2: Update 12 documentation files

Replace all occurrences of `new ServerCoordinator({...})` with `ServerFactory.create({...})` and update the corresponding `import { ServerCoordinator }` to `import { ServerFactory }` in the following files:

1. `apps/docs-astro/src/content/docs/reference/server.mdx` (1 occurrence)
2. `apps/docs-astro/src/content/docs/reference/adapter.mdx` (1 occurrence)
3. `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` (9 occurrences)
4. `apps/docs-astro/src/content/docs/guides/full-text-search.mdx` (2 occurrences)
5. `apps/docs-astro/src/content/docs/guides/performance.mdx` (3 occurrences)
6. `apps/docs-astro/src/content/docs/guides/deployment.mdx` (1 occurrence)
7. `apps/docs-astro/src/content/docs/guides/security.mdx` (1 occurrence)
8. `apps/docs-astro/src/content/docs/guides/authentication.mdx` (1 occurrence)
9. `apps/docs-astro/src/content/docs/guides/event-journal.mdx` (1 occurrence)
10. `apps/docs-astro/src/content/docs/guides/rbac.mdx` (1 occurrence)
11. `apps/docs-astro/src/content/docs/guides/interceptors.mdx` (3 occurrences)
12. `apps/docs-astro/src/content/blog/full-text-search-offline-first.mdx` (1 occurrence)

Total: 25 occurrences of `new ServerCoordinator(` to replace with `ServerFactory.create(`.

**Implementation notes for G2:**
- `interceptors.mdx` line 10 has a combined import: `import { ServerCoordinator, IInterceptor, ServerOp, OpContext }` — replace `ServerCoordinator` with `ServerFactory` within this destructured import (do NOT remove the other named exports).
- `full-text-search.mdx` line 594 (`serverSearchPermissionsCode`) and `full-text-search-offline-first.mdx` line 74 have `new ServerCoordinator(` WITHOUT an import statement in those code blocks — only the instantiation needs changing, there is no import line to update.
- `server.mdx` line 11 has `new ServerCoordinator(config)` in a template string WITHOUT an import — only the instantiation needs changing, there is no import line to update.
- **`server.mdx` prose updates:** Also update prose references to ServerCoordinator as the main API class:
  - Line 3 (frontmatter description): Change "The ServerCoordinator is the core of the TopGun backend" to "The Server API manages the TopGun backend"
  - Line 105 (body text): Change "The `ServerCoordinator` is the core of the TopGun backend." to "The `ServerFactory.create()` method creates a server instance that manages the TopGun backend."
  - Line 379 (method heading): Change "### constructor(config)" to "### ServerFactory.create(config)"
  - Line 111 (`ServerCoordinatorConfig` interface name) is correct — do NOT change.

## Requirements

### Files to Modify

| File | Change |
|------|--------|
| `packages/server/src/ProcessorSandbox.ts` | Replace `new Function()` + `Promise.race` timeout in `executeInFallback` with `vm.runInNewContext()` using the `timeout` option. Import `vm` from `node:vm`. |
| `packages/server/src/__tests__/ProcessorSandbox.test.ts` | Possibly adjust error message assertion if `vm.runInNewContext` timeout error wording differs from "timed out". Verify the test passes without `--forceExit`. |
| `apps/docs-astro/src/content/docs/reference/server.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import. Update prose at lines 3, 105, 379 to reflect `ServerFactory.create()` as the main API (see notes above). |
| `apps/docs-astro/src/content/docs/reference/adapter.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` | 9x `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/full-text-search.mdx` | 2x `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/performance.mdx` | 3x `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/deployment.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/security.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/authentication.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/event-journal.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/rbac.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import |
| `apps/docs-astro/src/content/docs/guides/interceptors.mdx` | 3x `new ServerCoordinator(` -> `ServerFactory.create(`, update combined import |
| `apps/docs-astro/src/content/blog/full-text-search-offline-first.mdx` | `new ServerCoordinator(` -> `ServerFactory.create(`, update import |

### Interfaces

No interface changes required. `ServerFactory.create()` already accepts the same `ServerCoordinatorConfig` that `new ServerCoordinator()` used to accept in the old API.

### Deletions

None.

## Goal Analysis

### Goal Statement

ProcessorSandbox tests run to completion without hanging, and all public documentation accurately reflects the current `ServerFactory.create()` server instantiation API.

### Observable Truths

1. **OT1:** `cd packages/server && npx jest --testPathPattern="ProcessorSandbox"` finishes within normal Jest timeout (no `--forceExit` required).
2. **OT2:** The "should timeout on infinite loop" test passes, returning `{ success: false, error: <contains "timed out"> }`.
3. **OT3:** `grep -r "new ServerCoordinator" apps/docs-astro/` returns 0 matches.
4. **OT4:** All documentation code examples that create a server use `import { ServerFactory } from '@topgunbuild/server'` and `ServerFactory.create({...})`.
5. **OT5:** `serverUrl` in client-side documentation examples remains unchanged (TopGunClient still accepts `serverUrl`).
6. **OT6:** Variable names in docs (e.g., `const server = ...`) remain the same — only the instantiation call changes.

### Required Artifacts

| Observable Truth | Artifact | Role |
|-----------------|----------|------|
| OT1, OT2 | `ProcessorSandbox.ts` | Fallback VM with proper timeout |
| OT1, OT2 | `ProcessorSandbox.test.ts` | Assertion alignment |
| OT3, OT4, OT5, OT6 | 12 `.mdx` files | Correct API usage |

### Key Links

- The fallback VM's synchronous execution blocks the event loop, preventing `Promise.race` timeout from firing. Using `vm.runInNewContext({ timeout })` delegates interruption to V8's native `TerminateExecution`, which works for synchronous code.
- The `vm` module's timeout error message may differ from the current "Processor execution timed out" string. The test assertion on line 265 (`expect(result.error).toContain('timed out')`) must match whatever error message the fixed code produces.

## Acceptance Criteria

1. **AC1:** `cd packages/server && npx jest --testPathPattern="ProcessorSandbox"` completes successfully (exit code 0) without `--forceExit` and without `--detectOpenHandles` warnings.
2. **AC2:** All existing ProcessorSandbox tests pass (basic execution, BuiltInProcessors, error handling, timeout, cache management, disposal, security mode, defaults).
3. **AC3:** The "should timeout on infinite loop" test verifies `result.success === false` and error message contains "timed out".
4. **AC4:** `grep -rn "new ServerCoordinator" apps/docs-astro/` returns 0 results.
5. **AC5:** `grep -rn "import.*ServerCoordinator.*from.*@topgunbuild/server" apps/docs-astro/` returns 0 results (all imports switched to `ServerFactory`).
6. **AC6:** No changes to `serverUrl` in any client-side documentation examples.
7. **AC7:** No changes to `ProcessorSandbox`'s public API (constructor signature, `execute`, `clearCache`, `getCacheStats`, `dispose`, `isSecureMode` remain identical).
8. **AC8:** The fallback script cache (`fallbackScriptCache`) continues to work — cached scripts are reused across invocations.
9. **AC9:** Existing server tests (`pnpm --filter @topgunbuild/server test`) continue to pass.

## Constraints

- Do NOT modify the `isolated-vm` path (`executeInIsolate`) — it is already correct.
- Do NOT change the `ProcessorSandbox` public API or constructor signature.
- Do NOT add `isolated-vm` as a required dependency.
- Do NOT touch `serverUrl` in client-side documentation examples.
- Do NOT modify variable names in documentation (e.g., keep `const server = ...`, `const node1 = ...`).
- Do NOT add phase/spec references in code comments.
- The `vm` module timeout approach must be compatible with Node.js 18+ (the project minimum).

## Assumptions

- **isolated-vm is not available in the test/CI environment.** The fallback path is what runs during testing. (Confirmed by the WARN log message in the task description.)
- **Node.js `vm.runInNewContext` with `timeout` option is sufficient** to interrupt synchronous infinite loops. This is documented Node.js behavior using V8's `TerminateExecution`.
- **The `vm` module's Script timeout error** throws an `Error` with a message like "Script execution timed out" — the catch block should normalize this to "Processor execution timed out" to match existing test expectations.
- **The 12 doc files listed are exhaustive.** No other files contain `new ServerCoordinator`.
- **`ServerFactory.create()` returns `ServerCoordinator`** and accepts the same config type (`ServerCoordinatorConfig`), so documentation examples need only change the instantiation line and import, not any subsequent usage of the `server` variable.
- **The blog file** `full-text-search-offline-first.mdx` follows the same pattern as guides and should be updated identically.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Fix `executeInFallback` in `ProcessorSandbox.ts`: replace `new Function()` + `Promise.race` with `vm.Script` + `runInNewContext({ timeout })`. Maintain fallback script caching by caching compiled `vm.Script` objects instead of raw functions. Normalize timeout error message to "Processor execution timed out". Since `vm.runInNewContext` is synchronous, remove the `async` keyword from the `executeInFallback` method signature. **Note:** Preserve the resolver cache bypass — processors whose name starts with 'resolver:' must not be cached (see lines 230-231 of current ProcessorSandbox.ts). When switching from caching Function objects to caching vm.Script objects, maintain this bypass behavior. | -- | ~12% |
| G2 | 1 | Update 12 documentation `.mdx` files: replace `import { ServerCoordinator }` with `import { ServerFactory }`, replace `new ServerCoordinator(` with `ServerFactory.create(`. See implementation notes above for edge cases (combined imports, missing imports). In `server.mdx`, also update prose references at lines 3, 105, 379 to reflect `ServerFactory.create()` as the main API. | -- | ~30% |
| G3 | 2 | Verify/adjust `ProcessorSandbox.test.ts` assertions if needed (error message wording, open handle cleanup). Run full test suite to confirm no regressions. | G1 | ~8% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-09)
**Status:** APPROVED

**Context Estimate:** ~50% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~50% | <=50% | ok |
| Largest task group | ~30% (G2) | <=30% | ok |
| Worker overhead | ~10% | <=10% | ok |

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
| OT1 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT2 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT3 has artifacts | ok | 12 .mdx files |
| OT4 has artifacts | ok | 12 .mdx files |
| OT5 has artifacts | ok | Constraint: no serverUrl changes |
| OT6 has artifacts | ok | Constraint: no variable name changes |
| Artifact wiring | ok | G1 -> G3 dependency covers test alignment |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | isolated-vm not available in test env | If available, the fallback path never runs and the bug doesn't manifest -- but the fix is still correct |
| A2 | vm.runInNewContext timeout interrupts synchronous code | If wrong (unlikely -- documented since Node.js 0.x), the fix won't work. Mitigation: this is well-established V8 behavior. |
| A3 | The 12 doc files are exhaustive | If wrong, some docs still reference old API. Mitigated by AC4/AC5 grep verification. |
| A4 | ServerFactory.create() returns ServerCoordinator | Confirmed by reading ServerFactory.ts line 58. |

**Strategic fit:** Aligned with project goals -- fixes a real test hang and brings docs up to date with the modular refactoring.

**Project compliance:** Honors PROJECT.md decisions -- no new dependencies, follows existing error handling pattern, uses WHY-comments convention.

**Comment:** Well-structured spec with clear root cause analysis, measurable acceptance criteria, and good separation of the two unrelated issues into parallel task groups. The implementation notes for G2 edge cases (combined imports, missing imports) were added during audit to prevent implementer confusion.

**Recommendations:**
1. [Minor] The `server.mdx` file contains prose references to `ServerCoordinator` at lines 3 ("description" frontmatter), 105 (body text: "The `ServerCoordinator` is the core..."), 111 (`ServerCoordinatorConfig`), and 379 (`### constructor(config)`) that describe it as the main API class. While these pass AC4/AC5 greps (which only check for `new ServerCoordinator` and import patterns), the prose is somewhat misleading since users now interact with `ServerFactory.create()`, not the constructor directly. Consider updating the prose in a follow-up or expanding this spec to cover it. The `ServerCoordinatorConfig` interface name is still correct (it is the actual type), so line 111 is fine.
2. [Minor] The `fallbackScriptCache` type will change from `Map<string, Function>` to `Map<string, vm.Script>` -- AC8 says "cached functions are reused" but should say "cached scripts are reused." The intent is clear but the wording is slightly imprecise.

### Response v1 (2026-02-09)
**Applied:** Both recommendations

**Changes:**
1. [✓] Recommendation #1 (server.mdx prose) — Expanded G2 scope to include prose updates at lines 3, 105, 379 in server.mdx. Added specific rewrite instructions in Implementation notes section. Updated Files to Modify table with additional changes for server.mdx.
2. [✓] Recommendation #2 (AC8 wording) — Changed AC8 from "cached functions are reused" to "cached scripts are reused" for precision.

**Notes:** Both recommendations address terminology precision without changing the implementation scope or acceptance criteria logic.

### Audit v2 (2026-02-09)
**Status:** APPROVED

**Context Estimate:** ~50% total

**Verification:** Re-audited with fresh eyes after v1 recommendations were applied. Cross-checked all spec claims against actual codebase:
- Confirmed 25 `new ServerCoordinator` occurrences across exactly 12 files in `apps/docs-astro/` (matches spec claim)
- Confirmed 12 `import.*ServerCoordinator.*from.*@topgunbuild/server` occurrences (matches spec's import update list)
- Confirmed `ServerFactory` is exported from `packages/server/src/index.ts` (line 65)
- Confirmed `ServerFactory.create()` returns `ServerCoordinator` and accepts `ServerCoordinatorConfig` (line 58 of ServerFactory.ts)
- Confirmed `executeInFallback` uses `new Function()` + `Promise.race`/`setTimeout` pattern (lines 235-253 of ProcessorSandbox.ts)
- Confirmed the test at line 251 of ProcessorSandbox.test.ts uses `while(true) {}` which would block the event loop
- Confirmed `server.mdx` prose at lines 3, 105, 379 matches the spec's description of what to change
- Confirmed `interceptors.mdx` line 10 has the combined import pattern described in implementation notes

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~50% | <=50% | ok |
| Largest task group | ~30% (G2) | <=30% | ok |
| Worker overhead | ~10% | <=10% | ok |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | Fix executeInFallback in ProcessorSandbox.ts | ~12% | 12% |
| G2 | 1 | Update 12 documentation .mdx files + server.mdx prose | ~30% | 42% |
| G3 | 2 | Verify/adjust test assertions, run full suite | ~8% | 50% |

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
| OT1 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT2 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT3 has artifacts | ok | 12 .mdx files |
| OT4 has artifacts | ok | 12 .mdx files |
| OT5 has artifacts | ok | Constraint: no serverUrl changes |
| OT6 has artifacts | ok | Constraint: no serverUrl changes |
| Artifact wiring | ok | G1 -> G3 dependency covers test alignment |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | isolated-vm not available in test env | If available, fallback path never runs -- fix is still correct |
| A2 | vm.runInNewContext timeout interrupts synchronous code | Well-established V8 behavior since Node.js 0.x |
| A3 | The 12 doc files are exhaustive | Verified: grep confirms exactly 12 files, 25 occurrences |
| A4 | ServerFactory.create() returns ServerCoordinator | Verified: ServerFactory.ts line 58 |

**Strategic fit:** Aligned with project goals -- fixes a real test hang and brings docs up to date with the modular refactoring.

**Project compliance:** Honors PROJECT.md decisions -- no new dependencies, no spec references in code, follows TypeScript strict mode, uses established patterns.

**Comment:** Spec is well-crafted and implementation-ready. Both v1 recommendations were properly applied. All claims verified against the actual codebase. The edge case documentation for G2 (combined imports, missing imports, prose updates) significantly reduces implementer ambiguity. The `executeInFallback` method is straightforward to fix with `vm.Script` + `runInNewContext({ timeout })`.

**Recommendations:**
1. [Minor] The `executeInFallback` method is currently declared `async` (returns `Promise<EntryProcessorResult<R>>`). After switching to `vm.runInNewContext` which is synchronous, the method no longer needs to be async. However, since the public `execute()` method already returns a Promise and both code paths (isolate and fallback) should maintain the same signature, keeping it async is harmless. The implementer can optionally remove the `async` keyword and return results directly, but this is not required.

### Response v2 (2026-02-09)
**Applied:** Recommendation #1

**Changes:**
1. [✓] Recommendation #1 (async keyword removal) — Added note to G1 task description: "Since `vm.runInNewContext` is synchronous, remove the `async` keyword from the `executeInFallback` method signature." This clarifies that the implementer should make the method synchronous after switching to the `vm` module.

**Notes:** This is a minor refinement that improves code clarity by removing unnecessary async wrapping after the switch from `Promise.race` to synchronous `vm.runInNewContext`.

### Audit v3 (2026-02-09)
**Status:** APPROVED

**Context Estimate:** ~50% total

**Fresh-eyes verification against codebase:**
- Confirmed `executeInFallback` (lines 223-275 of ProcessorSandbox.ts) uses `new Function()` + `Promise.race`/`setTimeout` -- the root cause analysis is accurate
- Confirmed the `while(true) {}` test at line 251 of ProcessorSandbox.test.ts would block the event loop, preventing `setTimeout` callback from firing
- Confirmed 25 `new ServerCoordinator` occurrences across exactly 12 files in `apps/docs-astro/` (grep count matches spec)
- Confirmed 12 `import.*ServerCoordinator.*from.*@topgunbuild/server` occurrences (matches spec)
- Confirmed `ServerFactory` exported from `packages/server/src/index.ts` line 65
- Confirmed `server.mdx` prose at lines 3, 105, 379 matches spec descriptions
- Confirmed `interceptors.mdx` line 10 combined import pattern
- Confirmed `full-text-search.mdx` line 594 and `full-text-search-offline-first.mdx` line 74 have no import statements (only instantiation)
- Confirmed `server.mdx` line 11 has `new ServerCoordinator(config)` in template string without import
- Confirmed resolver cache bypass logic at lines 230-231 of ProcessorSandbox.ts (`processor.name.startsWith('resolver:')` skips cache)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~50% | <=50% | ok |
| Largest task group | ~30% (G2) | <=30% | ok |
| Worker overhead | ~10% | <=10% | ok |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | Fix executeInFallback in ProcessorSandbox.ts | ~12% | 12% |
| G2 | 1 | Update 12 documentation .mdx files + server.mdx prose | ~30% | 42% |
| G3 | 2 | Verify/adjust test assertions, run full suite | ~8% | 50% |

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
| OT1 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT2 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT3 has artifacts | ok | 12 .mdx files |
| OT4 has artifacts | ok | 12 .mdx files |
| OT5 has artifacts | ok | Constraint: no serverUrl changes |
| OT6 has artifacts | ok | Constraint: no variable name changes |
| Artifact wiring | ok | G1 -> G3 dependency covers test alignment |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | isolated-vm not available in test env | If available, fallback path never runs -- fix is still correct |
| A2 | vm.runInNewContext timeout interrupts synchronous code | Well-established V8 behavior since Node.js 0.x |
| A3 | The 12 doc files are exhaustive | Verified: grep confirms exactly 12 files, 25 occurrences |
| A4 | ServerFactory.create() returns ServerCoordinator | Verified: ServerFactory.ts line 58 |

**Strategic fit:** Aligned with project goals -- fixes a real test hang and brings docs up to date with the modular refactoring.

**Project compliance:** Honors PROJECT.md decisions -- no new dependencies, no spec references in code, follows TypeScript strict mode, uses established patterns. Uses `node:vm` (built-in module, not a new dependency).

**Comment:** Spec is thorough and implementation-ready after two revision cycles. All claims verified against the actual codebase. The root cause analysis is accurate, the technical approach is sound, acceptance criteria are measurable, and edge cases for G2 are well-documented. Previous audit recommendations were properly applied.

**Recommendations:**
1. [Minor] The resolver cache bypass logic at line 230-231 of ProcessorSandbox.ts (`processor.name.startsWith('resolver:')` skips the `fallbackScriptCache`) is not explicitly mentioned in G1's task description. When switching from caching `Function` objects to caching `vm.Script` objects, the implementer should preserve this resolver bypass behavior. While AC2 ("all existing ProcessorSandbox tests pass") would catch a regression if there were resolver-specific tests, there are none in the current test file. Consider adding a note to G1: "Preserve the resolver cache bypass: processors whose name starts with 'resolver:' must not be cached."

### Response v3 (2026-02-09)
**Applied:** Recommendation #1

**Changes:**
1. [✓] Recommendation #1 (resolver cache bypass preservation) — Added note to G1 task description: "Preserve the resolver cache bypass — processors whose name starts with 'resolver:' must not be cached (see lines 230-231 of current ProcessorSandbox.ts). When switching from caching Function objects to caching vm.Script objects, maintain this bypass behavior." This ensures the implementer is aware of this existing behavior and preserves it during the refactoring.

**Notes:** This addresses a subtle but important implementation detail that could be overlooked during the Function-to-vm.Script migration. The resolver cache bypass is verified by manual inspection of the code but not explicitly tested, so documenting it in the task description provides necessary guidance.

### Audit v4 (2026-02-09 fresh-eyes)
**Status:** APPROVED

**Context Estimate:** ~50% total

**Independent verification against codebase (fresh auditor, no prior context):**
- Confirmed `executeInFallback` (lines 223-275 of ProcessorSandbox.ts) uses `new Function()` + `Promise.race`/`setTimeout` -- root cause analysis is accurate
- Confirmed `while(true) {}` test at line 251 of ProcessorSandbox.test.ts blocks event loop, preventing timeout callback
- Confirmed 25 `new ServerCoordinator(` occurrences across exactly 12 files in `apps/docs-astro/` (grep count matches spec)
- Confirmed 12 `import.*ServerCoordinator.*from.*@topgunbuild/server` occurrences across 10 files (2 files have instantiation without import: server.mdx and blog file -- matches spec notes)
- Confirmed `ServerFactory` exported from `packages/server/src/index.ts` line 65
- Confirmed `server.mdx` line 3 frontmatter, line 105 body text, line 379 heading match spec descriptions
- Confirmed `interceptors.mdx` line 10 combined import: `import { ServerCoordinator, IInterceptor, ServerOp, OpContext }`
- Confirmed `server.mdx` line 11 template string without import
- Confirmed resolver cache bypass at lines 230-231 (`processor.name.startsWith('resolver:')`)
- Confirmed `fallbackScriptCache` typed as `Map<string, Function>` at line 94

**Dimension Evaluation:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Pass | Root cause explained precisely. Task describes exactly what to do. No vague terms. |
| Completeness | Pass | All 14 files listed. Edge cases documented. Resolver bypass noted. Prose updates specified. |
| Testability | Pass | All 9 ACs are measurable via concrete commands or inspection. |
| Scope | Pass | Constraints clearly bound both parts. No scope creep. |
| Feasibility | Pass | `vm.runInNewContext({ timeout })` is well-documented Node.js API. Doc changes are mechanical. |
| Architecture fit | Pass | Uses built-in `node:vm`. Follows existing error normalization pattern from `executeInIsolate`. |
| Non-duplication | Pass | No reinvention. `vm` module is the correct tool for this. |
| Cognitive load | Pass | Solution is simpler than current `Promise.race` pattern. Doc changes are straightforward. |
| Strategic fit | Pass | Fixes real CI/test blocker. Aligns docs with current API. |
| Project compliance | Pass | No new dependencies. No spec references in code. TypeScript strict mode. WHY-comments convention. |

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~50% | <=50% | ok |
| Largest task group | ~30% (G2) | <=30% | ok |
| Worker overhead | ~10% | <=10% | ok |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | Fix executeInFallback in ProcessorSandbox.ts | ~12% | 12% |
| G2 | 1 | Update 12 documentation .mdx files + server.mdx prose | ~30% | 42% |
| G3 | 2 | Verify/adjust test assertions, run full suite | ~8% | 50% |

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
| OT1 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT2 has artifacts | ok | ProcessorSandbox.ts, test file |
| OT3 has artifacts | ok | 12 .mdx files |
| OT4 has artifacts | ok | 12 .mdx files |
| OT5 has artifacts | ok | Constraint: no serverUrl changes |
| OT6 has artifacts | ok | Constraint: no variable name changes |
| Artifact wiring | ok | G1 -> G3 dependency covers test alignment |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | isolated-vm not available in test env | If available, fallback path never runs -- fix is still correct |
| A2 | vm.runInNewContext timeout interrupts synchronous code | Well-established V8 behavior since Node.js 0.x |
| A3 | The 12 doc files are exhaustive | Independently verified: grep confirms exactly 12 files, 25 occurrences |
| A4 | ServerFactory.create() returns ServerCoordinator | Verified: ServerFactory exported from index.ts line 65 |

**Strategic fit:** Aligned with project goals -- fixes a real test hang and brings docs up to date with the modular refactoring.

**Project compliance:** Honors PROJECT.md decisions -- no new dependencies (node:vm is built-in), no spec references in code, follows TypeScript strict mode, uses established error normalization pattern.

**Comment:** This specification is implementation-ready. After 3 prior audit-revision cycles, all recommendations have been incorporated. The spec is thorough: root cause is precisely identified, the technical approach is sound, all 25 doc occurrences are enumerated with edge cases documented, acceptance criteria are concrete and verifiable, and the task groups are well-decomposed with correct dependency ordering. No critical issues found. No further recommendations -- the spec has been refined to a high standard.

## Execution Summary

**Executed:** 2026-02-10
**Mode:** orchestrated (sequential fallback)
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Modified
- `packages/server/src/ProcessorSandbox.ts` -- replaced `new Function()` + `Promise.race` with `vm.Script` + `runInNewContext({ timeout })`
- `apps/docs-astro/src/content/docs/reference/server.mdx` -- updated instantiation, import, and prose
- `apps/docs-astro/src/content/docs/reference/adapter.mdx` -- updated instantiation and import
- `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` -- updated 9 instantiations and import
- `apps/docs-astro/src/content/docs/guides/full-text-search.mdx` -- updated 2 instantiations and import
- `apps/docs-astro/src/content/docs/guides/performance.mdx` -- updated 3 instantiations and import
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` -- updated instantiation and import
- `apps/docs-astro/src/content/docs/guides/security.mdx` -- updated instantiation and import
- `apps/docs-astro/src/content/docs/guides/authentication.mdx` -- updated instantiation and import
- `apps/docs-astro/src/content/docs/guides/event-journal.mdx` -- updated instantiation and import
- `apps/docs-astro/src/content/docs/guides/rbac.mdx` -- updated instantiation, import, and prose
- `apps/docs-astro/src/content/docs/guides/interceptors.mdx` -- updated 3 instantiations and combined import
- `apps/docs-astro/src/content/blog/full-text-search-offline-first.mdx` -- updated instantiation

### Acceptance Criteria Status
- [x] AC1: `jest --testPathPattern="ProcessorSandbox"` completes successfully without `--forceExit`
- [x] AC2: All 27 ProcessorSandbox tests pass
- [x] AC3: Timeout test verifies `result.success === false` and error contains "timed out"
- [x] AC4: `grep -rn "new ServerCoordinator" apps/docs-astro/` returns 0 results
- [x] AC5: `grep -rn "import.*ServerCoordinator.*from.*@topgunbuild/server" apps/docs-astro/` returns 0 results
- [x] AC6: No changes to `serverUrl` in client-side docs
- [x] AC7: ProcessorSandbox public API unchanged (constructor, execute, clearCache, getCacheStats, dispose, isSecureMode)
- [x] AC8: Fallback script cache works -- cached vm.Script objects reused across invocations
- [x] AC9: Full server test suite passes (84 suites, 1187 tests)

### Deviations
- None

---

## Review History

### Review v1 (2026-02-10)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. `adapter.mdx` lines 296 and 300 still contain prose references to "ServerCoordinator" in the heading "### With ServerCoordinator" and description text "Pass the adapter to ServerCoordinator for persistent server storage." While the code block itself correctly uses `ServerFactory.create()`, the surrounding prose is slightly misleading for documentation readers. Not a spec violation (AC4/AC5 only check instantiation patterns and imports, which both pass).

2. `deployment.mdx` lines 520 and 522 contain prose references to "ServerCoordinator" in the serverless deployment section. These are contextually accurate (referring to the class itself rather than the old instantiation pattern) but could be refined for consistency. Not a spec violation.

**Passed:**
- [v] AC1: ProcessorSandbox tests complete without `--forceExit` -- `executeInFallback` now uses synchronous `vm.Script.runInNewContext({ timeout })` which uses V8's `TerminateExecution` to interrupt infinite loops
- [v] AC2: All 27 ProcessorSandbox tests pass -- basic execution, BuiltInProcessors, error handling, timeout, cache management, disposal, security mode, defaults
- [v] AC3: Timeout test at line 264-265 of test file verifies `result.success === false` and `result.error` contains "timed out" -- error normalization at `ProcessorSandbox.ts:278-282` converts "Script execution timed out" to "Processor execution timed out"
- [v] AC4: `grep -rn "new ServerCoordinator" apps/docs-astro/` returns 0 results -- verified
- [v] AC5: `grep -rn "import.*ServerCoordinator.*from.*@topgunbuild/server" apps/docs-astro/` returns 0 results -- verified
- [v] AC6: `serverUrl` unchanged in client-side docs -- 22 occurrences across 14 files, none modified
- [v] AC7: Public API unchanged -- constructor (line 98), execute (line 110), clearCache (line 346), getCacheStats (line 378), dispose (line 389), isSecureMode (line 371) all present with original signatures
- [v] AC8: Fallback script cache works -- `fallbackScriptCache` typed as `Map<string, vm.Script>` (line 95), resolver bypass preserved (lines 233-246), cache cleared in `clearCache()` and `dispose()`
- [v] AC9: Full server test suite passes per execution summary (84 suites, 1187 tests)
- [v] Constraint: `executeInIsolate` unchanged (lines 144-218)
- [v] Constraint: No `Promise.race` or `setTimeout` in ProcessorSandbox.ts
- [v] Constraint: No `new Function()` in ProcessorSandbox.ts
- [v] Constraint: `async` keyword removed from `executeInFallback` (line 226)
- [v] Constraint: Resolver cache bypass preserved -- processors starting with 'resolver:' skip cache (lines 233-246)
- [v] Constraint: Values passed directly to vm sandbox (not JSON.stringify), preserving `undefined` for `PUT_IF_ABSENT` semantics (lines 252-256)
- [v] Constraint: No spec/phase references in code comments
- [v] Constraint: Variable names in docs preserved (const server, const node1, const node2, etc.)
- [v] Constraint: `ServerCoordinatorConfig` at server.mdx line 111 NOT changed (correct -- it is the actual type name)
- [v] server.mdx prose at line 3, 105, 379 properly updated to reflect `ServerFactory.create()` API
- [v] interceptors.mdx combined import correctly updated: `ServerFactory, IInterceptor, ServerOp, OpContext`
- [v] Import `vm` from `node:vm` at line 6 -- built-in module, no new dependency
- [v] Code quality: clean, readable, follows existing error normalization pattern from `executeInIsolate`, good WHY-comments explaining vm timeout mechanism

**Summary:** Implementation is clean, correct, and fully compliant with all 9 acceptance criteria. The ProcessorSandbox fix uses the right approach (vm.Script with timeout) and is actually simpler than the previous Promise.race pattern. All 12 documentation files are properly updated with consistent ServerFactory.create() usage. Two minor prose references to "ServerCoordinator" remain in adapter.mdx and deployment.mdx but are outside spec scope and not misleading in context. No critical or major issues found.

---

## Completion

**Completed:** 2026-02-10
**Total Commits:** 3
**Audit Cycles:** 4
**Review Cycles:** 1
