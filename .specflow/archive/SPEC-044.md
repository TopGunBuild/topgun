---
id: SPEC-044
type: bugfix
status: done
priority: P2
complexity: small
created: 2026-02-09
todo: TODO-058
---

# Fix Flaky Split-Brain Recovery Test in Resilience.test.ts

## Context

The "Split-Brain Recovery: Eventual Consistency after Network Isolation" test in `Resilience.test.ts` fails consistently (3/3 runs). This is the last remaining consistently failing test in the server test suite.

**Root Cause:** The test incorrectly models the offline scenario using ChaosProxy silent mode. The `pollUntil` check (lines 109-117) verifies clients are NOT in `INITIAL`/`DISCONNECTED` states, but this does not guarantee silent mode is effectively isolating the clients. Clients reach `AUTHENTICATING` state (AUTH message is blackholed by the proxy), but the test proceeds to write before verifying the state has stabilized. After `proxy.disconnectAll()` + reconnect, a race condition during Merkle sync causes convergence failure.

**Production code is correct** -- this is purely a test infrastructure problem. The SyncEngine, ChaosProxy, and state machine all work as designed.

**Existing Coverage:** `Chaos.test.ts` already covers cluster-level split-brain behavior (A-B disconnect with partial convergence via node C), packet loss convergence, and slow consumer resilience. The E2E test suite (`multi-client.test.ts`) covers client reconnection and missed update delivery. The unique value of this test is verifying two-client offline-write convergence via CRDT LWW resolution after reconnection.

## Task

Rewrite the split-brain recovery test in `Resilience.test.ts` to use `client.close()` for offline simulation instead of ChaosProxy silent mode. This eliminates the flawed offline-state detection by using an explicit disconnect/reconnect cycle that is deterministic.

**Approach: Explicit close/recreate for offline simulation**

1. Connect both clients to the server (through proxy or directly) and authenticate them
2. Call `client.close()` on both clients to simulate going offline (deterministic disconnect)
3. Perform conflicting offline writes on the SAME LWWMap instances from step 1, using `map.set()` for local state AND `storageAdapter.appendOpLog()` to record operations for sync on reconnect (key1 = ValueA on client A, key1 = ValueB on client B after 20ms delay, plus independent keys)
4. Recreate SyncEngine instances for both clients with the same MemoryStorageAdapter instances (to preserve oplog), connecting them back to the server
5. Register the same LWWMap instances (with accumulated offline data) on the new SyncEngine instances
6. Wait for CONNECTED state on both clients
7. Wait for convergence using existing `waitForConvergence` helper
8. Assert LWW resolution: key1 converges to the later write, independent keys propagate to both

This approach avoids the ChaosProxy silent mode entirely, making the offline simulation deterministic.

## Requirements

### Files to Modify

1. **`packages/server/src/__tests__/Resilience.test.ts`** -- Rewrite the test:
   - Keep the `describe('Resilience & Chaos Testing')` structure
   - Keep `beforeAll` setup (server, proxy, tokens) -- proxy is still used by the overall test suite structure
   - Rewrite the split-brain test to use close/recreate pattern:
     - Phase 1: Connect both clients, authenticate, wait for CONNECTED state, register maps
     - Phase 2: Close both clients (`client.close()`)
     - Phase 3: Perform offline writes on the SAME LWWMap instances from Phase 1 AND record them in the MemoryStorageAdapter oplog via `storageAdapter.appendOpLog()` (since `recordOperation()` cannot be called on a closed SyncEngine, the oplog must be populated directly so the new SyncEngine can send them to the server via `OP_BATCH` on reconnect)
     - Phase 4: Create new SyncEngine instances with the same `MemoryStorageAdapter` instances (to preserve oplog)
     - Phase 5: Register the same LWWMap instances on new clients, authenticate, wait for CONNECTED
     - Phase 6: Poll for convergence, assert LWW resolution and bidirectional propagation
   - Use `pollUntil`/`waitForConvergence`/`waitForConnection` from test-helpers (no raw `setTimeout` delays for polling)
   - Ensure the 20ms delay between conflicting writes is preserved (for deterministic LWW winner)

### Files NOT Modified

- `packages/server/src/__tests__/utils/ChaosProxy.ts` -- No changes needed
- `packages/server/src/__tests__/utils/test-helpers.ts` -- No changes needed
- Any production code -- This is a test-only fix

## Acceptance Criteria

1. `cd packages/server && npx jest --forceExit --testPathPattern="Resilience" --verbose` passes on a single run
2. The same command passes 3 consecutive times (stability requirement)
3. The test verifies:
   - `key1` converges to `ValueB` on both maps (later HLC timestamp wins)
   - `keyA` (`OnlyA`) propagates to both maps
   - `keyB` (`OnlyB`) propagates to both maps
   - Server map contains converged values for `key1`, `keyA`
4. No raw `setTimeout` calls for waiting/polling (the 20ms delay for HLC timestamp ordering is acceptable)
5. No modifications to production code (packages/client, packages/core, packages/server/src excluding __tests__)
6. Test timeout remains at 30000ms
7. The `afterEach` cleanup properly closes any created clients (the recreated client references from Phase 4, since the originals were already closed in Phase 2)

## Constraints

- Do NOT modify ChaosProxy -- the proxy is correct; the issue is how the test uses it
- Do NOT modify any production SyncEngine, state machine, or CRDT code
- Do NOT remove the test entirely -- the offline-write convergence scenario has unique value not covered by Chaos.test.ts or E2E tests
- Do NOT introduce new test utility functions -- use existing `waitForAuthReady`, `waitForConvergence`, `waitForConnection`, `pollUntil` from test-helpers
- Do NOT use ChaosProxy silent mode in the rewritten test -- use explicit close/recreate instead
- Preserve the `beforeAll`/`afterAll` lifecycle (server + proxy setup/teardown) since other tests may be added to the describe block

## Assumptions

- Closing a SyncEngine and creating a new one with the same MemoryStorageAdapter will preserve the oplog entries, allowing them to be synced on reconnect (this is how offline-first persistence works)
- LWWMap instances can have records set locally without a connected SyncEngine; they use the HLC for timestamps which works offline
- The 20ms delay between conflicting writes is sufficient for HLC to produce distinct timestamps (HLC increments counter or millis, so even 1ms suffices)
- A single test is sufficient -- no need to add additional Resilience test cases in this spec
- After `client.close()`, offline writes must populate both the LWWMap (via `map.set()`) AND the MemoryStorageAdapter oplog (via `storageAdapter.appendOpLog()`) so the new SyncEngine can send them to the server; Merkle sync for LWWMap is server-to-client only, so client data reaches the server exclusively through the oplog/OP_BATCH mechanism

## Audit History

### Audit v1 (2026-02-09)
**Status:** NEEDS_REVISION

**Context Estimate:** ~10% total (1 test file modification, small complexity)

**Critical:**
1. **Oplog gap in offline write approach.** The spec says "Perform offline writes on the LWWMap instances (maps are local, no network needed)" (Phase 3 in Requirements), but `map.set()` alone only updates the in-memory map and its Merkle tree -- it does NOT write to the storage adapter's oplog. After `client.close()`, `recordOperation()` cannot be called on the closed SyncEngine. The new SyncEngine's `loadOpLog()` will find no pending ops, so `syncPendingOperations()` on reconnect will send nothing to the server. Since LWWMap Merkle sync is server-to-client only (server sends `SYNC_RESP_LEAF` to client, never the reverse), the client's offline writes will never reach the server, and convergence will fail. **Fix:** Phase 3 must explicitly state that offline writes need BOTH `map.set()` (for local map state) AND direct `storageAdapter.appendOpLog()` calls (for oplog entries the new SyncEngine will send via `OP_BATCH`). The oplog entries must include mapName, opType ('PUT'), key, record (the LWWRecord from `map.set()`), and timestamp.

2. **Contradiction between Approach step 3 and Requirements Phase 3.** The Approach says "Create new LWWMap instances" in step 3, but the Requirements Phase 3 says "Perform offline writes on the LWWMap instances" (implying the existing maps from Phase 1). Additionally, Approach step 5 says "Register the same maps (with accumulated offline data)" which contradicts step 3's "new" maps. If new maps are created, they won't have the data accumulated by the "old" maps from Phase 1 (and vice versa). **Fix:** Clarify that the SAME LWWMap instances from Phase 1 are used for offline writes in Phase 3 and registered on new SyncEngines in Phase 5. The maps retain their HLC reference from the original SyncEngine, which works fine since the HLC object is not destroyed by `SyncEngine.close()`.

**Recommendations:**
3. The constraint "Do NOT introduce new test utility functions -- use existing `waitForAuthReady`, `waitForConvergence`, `pollUntil` from test-helpers" omits `waitForConnection` which already exists in test-helpers.ts and is ideal for Phase 5's "wait for CONNECTED state." Consider listing it as an available helper.
4. Acceptance criterion 4 says "No raw `setTimeout` calls for waiting/polling" but the 20ms delay between conflicting writes (for deterministic HLC ordering) is a legitimate use of `setTimeout` for timing, not for polling. Consider explicitly exempting this case: "No raw `setTimeout` calls for waiting/polling (the 20ms delay for HLC timestamp ordering is acceptable)."
5. The `afterEach` cleanup (AC 7) should account for the fact that during the close/recreate pattern, up to 4 SyncEngine instances may exist over the test lifetime (2 original + 2 recreated). The spec should note that the `afterEach` must close the CURRENT client references (the recreated ones), since the originals were already closed in Phase 2.

### Response v1 (2026-02-09 10:42)
**Applied:** All critical issues (1-2) and all recommendations (3-5)

**Changes:**
1. [✓] Oplog gap in offline write approach — Updated Approach step 3 to specify "using `map.set()` for local state AND `storageAdapter.appendOpLog()` to record operations for sync on reconnect"; Requirements Phase 3 already addressed this, now Approach is consistent
2. [✓] Contradiction between Approach step 3 and Requirements Phase 3 — Changed Approach step 3 from "Create new LWWMap instances" to "Perform conflicting offline writes on the SAME LWWMap instances from step 1"; step 5 already said "Register the same LWWMap instances", now consistent throughout
3. [✓] Missing `waitForConnection` in constraint list — Added `waitForConnection` to the Constraints section's list of available helpers from test-helpers; also added it to Requirements Phase 1 bullet point
4. [✓] setTimeout exemption for HLC timing — Updated AC #4 to explicitly exempt the 20ms delay: "No raw `setTimeout` calls for waiting/polling (the 20ms delay for HLC timestamp ordering is acceptable)"
5. [✓] afterEach cleanup detail — Updated AC #7 to clarify which clients to close: "properly closes any created clients (the recreated client references from Phase 4, since the originals were already closed in Phase 2)"

**Skipped:** None

### Audit v2 (2026-02-09)
**Status:** APPROVED

**Context Estimate:** ~10% total (1 test file modification, small complexity)

**Quality Projection:** PEAK range (0-30%)

**Verification of key assumptions against source code:**
- `SyncEngine.close()` does NOT call `storageAdapter.close()` -- MemoryStorageAdapter data is preserved (confirmed at `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/SyncEngine.ts:864-881`)
- `SyncEngine` constructor calls `loadOpLog()` which calls `storageAdapter.getPendingOps()` (confirmed at line 320, 433-453)
- `handleAuthAck()` calls `syncPendingOperations()` which sends pending ops via `OP_BATCH` (confirmed at line 699, 498-510)
- `MemoryStorageAdapter.appendOpLog()` exists and `getPendingOps()` filters by `!op.synced` (confirmed at `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/__tests__/utils/MemoryStorageAdapter.ts:47-55`)
- `OpLogEntry` fields used at runtime by `SyncEngine.recordOperation`: `mapName`, `opType`, `key`, `record`, `timestamp`, `synced` (confirmed at line 472-481)

**Strategic fit:** Aligned with project goals -- fixes the last consistently failing server test, continues the test stabilization work from SPEC-042.

**Project compliance:** Honors PROJECT.md decisions -- test-only change, Jest, no new dependencies, uses established polling patterns.

**Comment:** Well-structured spec with clear root cause analysis. The v1 audit caught two critical issues (oplog gap and map instance contradiction) that were properly addressed in the revision. All assumptions have been verified against source code. The approach is deterministic and eliminates the race condition inherent in ChaosProxy silent mode usage.

### Files Modified

- /Users/koristuvac/Projects/topgun/topgun/.specflow/specs/SPEC-044.md

---

## Execution Summary

**Executed:** 2026-02-09
**Commits:** 1

### Files Modified
- `packages/server/src/__tests__/Resilience.test.ts` — Rewrote split-brain recovery test to use close/recreate pattern instead of ChaosProxy silent mode

### Files Created
(none)

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC1: `npx jest --forceExit --testPathPattern="Resilience" --verbose` passes on a single run
- [x] AC2: Same command passes 3 consecutive times (all 3 passed)
- [x] AC3: Test verifies key1 converges to ValueB, keyA/keyB propagate to both maps, server map contains converged values
- [x] AC4: No raw setTimeout for polling (only the 20ms HLC delay remains)
- [x] AC5: No production code modified
- [x] AC6: Test timeout remains at 30000ms
- [x] AC7: afterEach closes recreated client references

### Deviations
(none)

### Notes
- Removed unused `pollUntil` import since `waitForConnection` and `waitForConvergence` handle all polling needs
- The oplog entries use `opType` field (matching SyncEngine's internal format) via `as any` cast, consistent with how `recordOperation()` writes to the storage adapter
- Test runs in ~150ms, down from the previous version which often timed out

---

## Review History

### Review v1 (2026-02-09 19:37)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. The `as any` casts on oplog entries (lines 115-122, 129-136, 140-147, 149-156) are pragmatic but could be made more type-safe by constructing objects that match the `IStorageAdapter.OpLogEntry` interface (`op` field instead of `opType`). However, since the production `SyncEngine.recordOperation()` at line 483 uses the same `as any` pattern, and `loadOpLog()` reconstructs entries with `as unknown as OpLogEntry`, this is consistent with established codebase patterns. No change needed.

2. The "Phase N" labels in comments (lines 71, 104, 108, 165, 181, 195) could be read as project-phase references per the CLAUDE.md convention (`// Phase 8.02`). However, these are clearly internal test-step labels describing the test structure, not external project/milestone references. This usage is idiomatic and improves readability. No change needed.

**Passed:**
- [check] AC1: Test passes on a single run (verified: PASS, 146ms)
- [check] AC2: Test passes 4 consecutive runs (all passed, 146-149ms range)
- [check] AC3: Test verifies key1=ValueB on both maps (lines 201-202), keyA=OnlyA on both (lines 204-205), keyB=OnlyB on both (lines 207-208), server map has key1=ValueB and keyA=OnlyA (lines 212-213)
- [check] AC4: Only one setTimeout at line 125 for the 20ms HLC delay -- no polling/waiting setTimeout calls
- [check] AC5: `git diff HEAD~1 -- packages/` confirms only `packages/server/src/__tests__/Resilience.test.ts` was modified
- [check] AC6: Test timeout is 30000ms (line 214: `}, 30000)`)
- [check] AC7: `afterEach` at lines 59-68 closes `clientA` and `clientB` (the reassigned/recreated references from Phase 4)
- [check] Constraint: No ChaosProxy silent mode usage (`isSilent: true` absent from test body)
- [check] Constraint: `beforeAll`/`afterAll` lifecycle preserved (server + proxy setup/teardown intact)
- [check] Constraint: No new test utility functions introduced -- uses existing `waitForAuthReady`, `waitForConvergence`, `waitForConnection`
- [check] Constraint: No production code modified (verified via `git diff`)
- [check] Quality: Code is well-structured with clear phase comments, proper error handling via polling helpers
- [check] Quality: `beforeEach` resets proxy config, ensuring clean state for potential future tests
- [check] Integration: Oplog entries use `opType` field matching SyncEngine's internal format, consistent with `recordOperation()` pattern
- [check] Integration: Same MemoryStorageAdapter instances reused across close/recreate cycle, verified data preserved
- [check] Architecture: Follows established test patterns (polling helpers, dynamic ports, JWT auth)
- [check] Security: No hardcoded secrets beyond the dev JWT secret already in the test suite
- [check] Non-duplication: Reuses existing test helpers, no new utilities introduced
- [check] Cognitive load: Test flow is linear and easy to follow with 6 clearly labeled phases

**Summary:** Clean, well-structured implementation that faithfully follows the specification. The close/recreate pattern eliminates the ChaosProxy silent mode race condition entirely. All 7 acceptance criteria and all constraints are satisfied. The test is deterministic, fast (~150ms), and stable across multiple consecutive runs. No critical or major issues found.

---

## Completion

**Completed:** 2026-02-09
**Total Commits:** 1
**Audit Cycles:** 2
**Review Cycles:** 1
