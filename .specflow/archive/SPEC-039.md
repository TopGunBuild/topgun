---
id: SPEC-039
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-02-08
source: TODO-056
---

# Add Reject Path to network.start() Promise for Listen Failure Handling

## Context

`network.start()` in `packages/server/src/modules/network-module.ts` creates a `new Promise<number>((resolve) => {...})` with no `reject` path. If `httpServer.listen()` encounters an error (e.g., EADDRINUSE — port already in use), the listen callback never fires, the Promise never resolves, and `server.ready()` hangs indefinitely.

This was identified as a pre-existing pattern limitation during the SPEC-038 review (see STATE.md decision log: "1 minor pre-existing pattern limitation (network.start() Promise never rejects)").

Currently, `httpServer.on('error')` is registered in `ServerCoordinator` (line 341) but only logs the error. It does not unblock the startup Promise chain. Meanwhile, `cluster.start()` already has a `.catch()` handler in `ServerFactory.ts` (line 433), demonstrating the expected error-handling pattern. The network startup path lacks the same robustness.

## Task

Add a `reject` callback to the `network.start()` Promise so that listen failures (EADDRINUSE, EACCES, etc.) reject the Promise instead of hanging. Propagate the rejection through the `Promise.all` startup chain in `ServerFactory.create()` so that `coordinator.ready()` rejects with a meaningful error. Add a `.catch()` handler on the `Promise.all` in `ServerFactory` to log the failure and reject the coordinator's ready promise.

## Requirements

### Files to Modify

1. **`packages/server/src/modules/network-module.ts`** (line 94-101)
   - Change `new Promise<number>((resolve) => {...})` to `new Promise<number>((resolve, reject) => {...})`
   - Add `httpServer.on('error', reject)` inside the Promise constructor, before `httpServer.listen()`
   - Remove the error listener after successful listen to avoid duplicate handling (the ServerCoordinator registers its own `httpServer.on('error')` for runtime errors)

2. **`packages/server/src/ServerFactory.ts`** (line 438)
   - Add `.catch()` on the `Promise.all([networkReady, clusterReady])` chain
   - In the catch handler, call `coordinator.failStartup(err)` (new method) so that `ready()` rejects
   - Log the startup failure with `logger.error`

3. **`packages/server/src/ServerCoordinator.ts`**
   - Change `_readyPromise` to also store a `_readyReject` callback (alongside existing `_readyResolve`)
   - Add `failStartup(err: Error): void` method that calls `_readyReject(err)` and logs the error
   - Ensure `_readyPromise` constructor uses `new Promise<void>((resolve, reject) => {...})`

### Interfaces

No new types or interfaces required. The `NetworkModule.start` return type remains `Promise<number>` (now it can reject).

## Acceptance Criteria

1. **network.start() rejects on listen error:** When `httpServer.listen()` fails (e.g., EADDRINUSE), the Promise returned by `network.start()` rejects with the underlying Node.js error.

2. **Error listener is removed after success:** After a successful listen, the one-time error listener registered for startup rejection is removed from `httpServer` to avoid interfering with the runtime error handler in `ServerCoordinator`.

3. **Promise.all propagates rejection:** The `Promise.all([networkReady, clusterReady])` in `ServerFactory.create()` has a `.catch()` that invokes `coordinator.failStartup(err)`.

4. **coordinator.ready() rejects:** When `failStartup(err)` is called, the Promise returned by `coordinator.ready()` rejects with the startup error.

5. **Startup failure is logged:** The failure is logged via `logger.error` with the error details and a message indicating the server failed to start.

6. **Successful startup is unaffected:** When the port is available, `network.start()` resolves with the actual port, `completeStartup()` is called, and `ready()` resolves -- identical to current behavior.

7. **Cluster error handling is preserved:** The existing `.catch()` on `cluster.start()` continues to work as before (logs and returns fallback port).

## Constraints

- Do NOT change the `ServerCoordinator` constructor signature or `ServerDependencies` interface.
- Do NOT add a `.catch()` that swallows the network error silently -- the rejection must propagate to `ready()`.
- Do NOT modify runtime error handling (`httpServer.on('error')` in `ServerCoordinator` constructor) -- that handler is for errors after successful listen.
- Keep the fix minimal: three files, focused on the startup Promise chain only.

## Assumptions

- The `httpServer.on('error', reject)` listener should be registered inside the Promise constructor and removed after successful listen via `httpServer.removeListener('error', reject)` in the listen callback. This prevents the startup rejection handler from firing on runtime errors that occur after the server is already listening.
- The `failStartup` method on `ServerCoordinator` is the cleanest way to signal ready-promise rejection, mirroring the existing `completeStartup` pattern.
- No test file modifications are required since existing tests use available ports (port 0 or high-range ports). A new test could verify the EADDRINUSE scenario but is not mandatory for this bugfix scope.

## Audit History

### Audit v1 (2026-02-08)
**Status:** APPROVED

**Context Estimate:** ~16% total

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions:**

| Dimension | Status |
|-----------|--------|
| Clarity | Pass |
| Completeness | Pass |
| Testability | Pass |
| Scope | Pass |
| Feasibility | Pass |
| Architecture fit | Pass |
| Non-duplication | Pass |
| Cognitive load | Pass |
| Strategic fit | Pass -- genuine bug fix for production hang scenario |
| Project compliance | Pass -- honors PROJECT.md decisions |

**Source Verification:**
- network-module.ts line 95: Confirmed `new Promise<number>((resolve) => {...})` with no reject path
- ServerFactory.ts line 438: Confirmed `Promise.all` with no `.catch()`
- ServerFactory.ts line 433: Confirmed `cluster.start().catch()` exists as reference pattern
- ServerCoordinator.ts line 208: Confirmed `_readyPromise = new Promise((resolve) => {...})` with no reject
- ServerCoordinator.ts line 341: Confirmed `httpServer.on('error')` only logs, does not unblock startup
- ServerCoordinator.ts line 371: Confirmed `completeStartup` pattern that `failStartup` mirrors

**Assumptions verified:** All 3 assumptions are sound. Node.js `httpServer.on('error')` fires synchronously before listen callback on EADDRINUSE, `removeListener` is standard practice, and existing tests use port 0 so they are unaffected.

**Comment:** Excellent specification. Well-scoped bugfix with precise line references (all verified against source), clear acceptance criteria, and a clean symmetrical design (failStartup mirrors completeStartup). The spec correctly identifies the existing cluster.start().catch() as the reference pattern. No critical or recommendation-level issues found.

---

## Execution Summary

**Executed:** 2026-02-08
**Commits:** 1

### Files Modified
- `packages/server/src/modules/network-module.ts` -- Added reject callback to network.start() Promise; registers one-time error listener before httpServer.listen() and removes it after successful listen
- `packages/server/src/ServerFactory.ts` -- Added .catch() on Promise.all([networkReady, clusterReady]) that calls coordinator.failStartup(err) and logs the failure
- `packages/server/src/ServerCoordinator.ts` -- Added _readyReject callback to _readyPromise constructor; added failStartup(err) public method that rejects ready() and logs the error

### Acceptance Criteria Status
- [x] AC1: network.start() rejects on listen error -- onStartupError listener registered before httpServer.listen(), calls reject(err)
- [x] AC2: Error listener is removed after success -- httpServer.removeListener('error', onStartupError) called in listen callback
- [x] AC3: Promise.all propagates rejection -- .catch() added that calls coordinator.failStartup(err)
- [x] AC4: coordinator.ready() rejects -- failStartup calls _readyReject(err)
- [x] AC5: Startup failure is logged -- logger.error in both ServerFactory .catch() and ServerCoordinator.failStartup()
- [x] AC6: Successful startup is unaffected -- resolve path unchanged, error listener removed after success
- [x] AC7: Cluster error handling is preserved -- existing cluster.start().catch() untouched

### Deviations
None.

### Notes
- The onStartupError callback wraps reject in a named function for clean removeListener() semantics
- The .catch() in ServerFactory normalizes non-Error values with `err instanceof Error ? err : new Error(String(err))` for type safety
- Pre-existing test failures (Resilience, Chaos, SearchCoordinator.batching) are unrelated to this change
- Build and all relevant tests pass

---

## Review History

### Review v1 (2026-02-08)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC1: network.start() rejects on listen error -- `onStartupError` listener registered via `httpServer.on('error', onStartupError)` at line 103 of `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/modules/network-module.ts`, before `httpServer.listen()` at line 105. The callback calls `reject(err)` at line 101.
- [x] AC2: Error listener is removed after success -- `httpServer.removeListener('error', onStartupError)` called at line 106 inside the listen callback, preventing the startup handler from interfering with the runtime error handler in ServerCoordinator (line 343).
- [x] AC3: Promise.all propagates rejection -- `.catch()` added at line 440 of `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/ServerFactory.ts`, calls `coordinator.failStartup(err)` with proper error normalization.
- [x] AC4: coordinator.ready() rejects -- `failStartup` at line 373 of `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/ServerCoordinator.ts` calls `this._readyReject(err)`, and `_readyReject` is wired to the Promise's `reject` callback at line 211.
- [x] AC5: Startup failure is logged -- `logger.error` called in both `ServerFactory.ts:441` ("Server startup failed") and `ServerCoordinator.ts:374` ("Server failed to start").
- [x] AC6: Successful startup is unaffected -- The resolve path is unchanged: `resolve(actualPort)` at line 109 of network-module.ts, `completeStartup` at line 439 of ServerFactory.ts, `_readyResolve()` at line 390 of ServerCoordinator.ts. Error listener is removed before resolve so it cannot interfere.
- [x] AC7: Cluster error handling is preserved -- `cluster.start().catch()` at ServerFactory.ts line 433-436 is unchanged and still returns a fallback port.
- [x] Constraint: ServerCoordinator constructor signature unchanged (verified via git diff -- no changes to constructor parameters).
- [x] Constraint: ServerDependencies interface unchanged (verified -- no diff on ServerDependencies.ts).
- [x] Constraint: Runtime error handler in ServerCoordinator (line 343) is untouched (verified by reading the code).
- [x] Constraint: Minimal fix -- exactly 3 files modified, focused on startup Promise chain only.
- [x] No spec/bug/phase references in code comments (verified via grep).
- [x] Build passes (tsup ESM + DTS success).
- [x] Relevant tests pass (GracefulShutdown, HandlersModule, HttpSyncEndpoint, Phase3Integration -- all green).
- [x] Code follows existing patterns -- `failStartup` mirrors `completeStartup` symmetrically; named error callback with `removeListener` is idiomatic Node.js.
- [x] Error normalization in ServerFactory `.catch()` (`err instanceof Error ? err : new Error(String(err))`) is defensive and type-safe.
- [x] No security concerns -- no new inputs, no secrets, no user-facing changes.
- [x] No duplication -- reuses existing logger, existing Promise pattern, existing coordinator method style.
- [x] Cognitive load is low -- the change is immediately understandable to any Node.js developer.

**Summary:** Clean, minimal, and correct implementation. All 7 acceptance criteria are met. All 4 constraints are respected. The code follows established project patterns (deferred startup, late binding, WHY-comments) and introduces no regressions. The `failStartup`/`completeStartup` symmetry is elegant. No issues found.

---

## Completion

**Completed:** 2026-02-08
**Total Commits:** 1
**Audit Cycles:** 1
**Review Cycles:** 1
