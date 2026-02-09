---
id: SPEC-043
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-02-09
---

# Fix SearchCoordinator Batched LEAVE Notification Bug

## Context

`SearchCoordinator.notifySubscribers()` (lines 759-836) mutates `subscription.currentResults` unconditionally -- even when no `sendUpdate` callback is registered and the notification is effectively discarded. When only a `sendBatchUpdate` callback is configured, the immediate path in `notifySubscribers()` deletes the key from `currentResults` (line 804) but never delivers the LEAVE notification (because `this.sendUpdate` is undefined). When the batched path later runs via `computeSubscriptionUpdate()` (line 1031), it checks `subscription.currentResults.has(key)` which now returns `false` because the immediate path already consumed the state. The result: batched LEAVE notifications are silently dropped.

**Root Cause:** `notifySubscribers()` mutates `currentResults` regardless of whether the notification is actually delivered. The `currentResults.delete(key)` on line 804 and `currentResults.set(key, ...)` on lines 801, 811 execute even when neither `sendUpdate` nor the distributed path can deliver the update.

**Impact:** Any subscription using the batched notification path (`queueNotification` + `sendBatchUpdate`) will never receive LEAVE updates when a document stops matching the query. ENTER and UPDATE work because they either add new state or modify existing state that persists for the batch path to read.

**Failing Test:** `SearchCoordinator.batching.test.ts` line 376 -- "should detect LEAVE when document no longer matches" (expects 1 LEAVE update, receives 0).

## Task

Fix `notifySubscribers()` so it does NOT mutate `subscription.currentResults` when the notification cannot be delivered (no `sendUpdate` callback and not a distributed subscription). This preserves state for `computeSubscriptionUpdate()` to correctly detect LEAVE transitions in the batched path.

## Requirements

### Modified Files

**`packages/server/src/search/SearchCoordinator.ts`**

In the `notifySubscribers()` method (lines 759-836):

1. Guard `currentResults` mutations so they only execute when the notification will actually be delivered. Specifically, the three mutation sites are:
   - Line 801: `sub.currentResults.set(key, { score: newScore, matchedTerms })` (ENTER)
   - Line 804: `sub.currentResults.delete(key)` (LEAVE)
   - Line 811: `sub.currentResults.set(key, { score: newScore, matchedTerms })` (UPDATE)

2. These mutations must only occur when EITHER:
   - `sub.isDistributed && sub.coordinatorNodeId` is truthy (distributed path delivers the update), OR
   - `this.sendUpdate` is defined (immediate path delivers the update)

3. When neither delivery path exists, `notifySubscribers()` must skip `currentResults` mutations entirely, leaving the state intact for the batched path in `computeSubscriptionUpdate()` to process.

4. The `computeSubscriptionUpdate()` method (lines 1024-1072) requires NO changes -- it already correctly reads and mutates `currentResults` and produces the proper update types.

### No New Files

This is a single-method fix in one file. No new files, interfaces, or types are needed.

## Acceptance Criteria

- **AC-1:** The test "should detect LEAVE when document no longer matches" (line 376) passes: receives exactly 1 LEAVE update for key `doc-1`.
- **AC-2:** The test "should handle remove changeType" (line 383) continues to pass: receives exactly 1 LEAVE update.
- **AC-3:** All other tests in `SearchCoordinator.batching.test.ts` continue to pass (8 total tests in file).
- **AC-4:** The immediate notification path (when `sendUpdate` IS set and `sendBatchUpdate` is NOT set) continues to work: `notifySubscribers()` mutates `currentResults` and delivers updates via `sendUpdate`.
- **AC-5:** The distributed subscription path continues to work: `notifySubscribers()` mutates `currentResults` and emits `distributedUpdate` event.
- **AC-6:** When BOTH `sendUpdate` and `sendBatchUpdate` are set, `onDataChange` uses the immediate path (which mutates `currentResults`), and `queueNotification` uses the batched path. The test "should support both immediate and batch callbacks simultaneously" (line 480) continues to pass.
- **AC-7:** Full SearchCoordinator test suite passes: `cd packages/server && npx jest --forceExit --testPathPattern="SearchCoordinator"`.

## Constraints

- Do NOT modify `computeSubscriptionUpdate()` -- it is already correct.
- Do NOT change the `SearchSubscription` interface or add new fields.
- Do NOT change the public API of `SearchCoordinator` (no new methods, no signature changes).
- Do NOT add spec/bug references in code comments -- use WHY-comments explaining the reason.
- Keep the fix minimal: only guard the three `currentResults` mutation lines inside `notifySubscribers()`.

## Assumptions

- The `onDataChange` -> `notifySubscribers` -> immediate path is the ONLY code path that mutates `currentResults` outside of `computeSubscriptionUpdate`. No other methods modify `currentResults` between notify and batch flush. (Verified by reading the full source.)
- The "handle remove changeType" test (line 383) works currently because `queueNotification` with `changeType: 'remove'` goes directly to the batched path (since `sendBatchUpdate` is set), and `onDataChange` for the seed document on line 394 uses `notifySubscribers` which has no `sendUpdate` set -- but the test does NOT call `onDataChange` with remove before queueing, so `currentResults` is still intact when the batch processes. (No collision with the immediate path for this specific test case.)

## Audit History

### Audit v1 (2026-02-09)
**Status:** APPROVED

**Context Estimate:** ~11% total

**Quality Projection:** PEAK range (0-30%)

**Dimensions:**

| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Root cause, impact, and fix are precisely described with line numbers |
| Completeness | Pass | Single file, three mutation sites, guard condition fully specified |
| Testability | Pass | 7 acceptance criteria, each measurable with specific test references |
| Scope | Pass | Minimal fix, clear boundaries, no scope creep |
| Feasibility | Pass | Verified against source -- bug analysis is accurate, fix approach is correct |
| Architecture Fit | Pass | No new patterns, conditional guard within existing method |
| Non-Duplication | Pass | Fixing existing logic, nothing duplicated |
| Cognitive Load | Pass | Simple guard condition, easy to understand and maintain |
| Strategic Fit | Pass | P1 bugfix for silent data loss -- appropriate priority |
| Project Compliance | Pass | Honors WHY-comments convention, no new deps, no API changes |

**Assumptions Validated:**

| # | Assumption | Validation |
|---|------------|------------|
| A1 | `notifySubscribers` is the only external mutator of `currentResults` | Confirmed via grep: only `notifySubscribers` (L801,804,811) and `computeSubscriptionUpdate` (L1049,1052,1057) mutate it, plus initial population during subscribe (L503,657) |
| A2 | "handle remove changeType" test does not collide with immediate path | Confirmed: test only uses `queueNotification` (no `onDataChange` with remove), so `currentResults` is intact when batch processes |

**Strategic fit:** Aligned with project goals -- fixes real bug with failing test, minimal risk.

**Project compliance:** Honors PROJECT.md decisions (no new deps, WHY-comments, TypeScript strict).

**Comment:** Excellent specification. The root cause analysis is thorough and accurate, verified against the actual source code. The guard condition logic is correct: mutations should only occur when a delivery path exists (distributed or immediate). The three mutation sites at lines 801, 804, and 811 are accurately identified. All 7 acceptance criteria are concrete and verifiable. The assumptions section demonstrates careful analysis of test interactions. This is ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-09
**Commits:** 1

### Files Modified
- `packages/server/src/search/SearchCoordinator.ts` -- Added `canDeliver` guard around the three `currentResults` mutation sites in `notifySubscribers()`. Mutations now only execute when either the distributed path (`sub.isDistributed && sub.coordinatorNodeId`) or the immediate path (`this.sendUpdate`) can deliver the update.

### Acceptance Criteria Status
- [x] AC-1: Test "should detect LEAVE when document no longer matches" passes (1 LEAVE update for doc-1)
- [x] AC-2: Test "should handle remove changeType" continues to pass (1 LEAVE update)
- [x] AC-3: All 16 tests in SearchCoordinator.batching.test.ts pass
- [x] AC-4: Immediate notification path works (canDeliver=true when sendUpdate is set)
- [x] AC-5: Distributed subscription path works (canDeliver=true when isDistributed && coordinatorNodeId)
- [x] AC-6: Both immediate and batch callbacks work simultaneously (test passes)
- [x] AC-7: Full SearchCoordinator test suite passes (4 suites, 67 tests, 0 failures)

### Deviations
None.

### Notes
The fix adds 14 lines and removes 3, resulting in a net +11 lines. The `canDeliver` boolean is computed once per subscription iteration and reused across all three guard sites. No changes to `computeSubscriptionUpdate()`, `SearchSubscription` interface, or public API.

---

## Review History

### Review v1 (2026-02-09)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC-1: Test "should detect LEAVE when document no longer matches" (line 349) passes -- confirmed via test run, 1 LEAVE update for `doc-1`
- [x] AC-2: Test "should handle remove changeType" (line 383) passes -- confirmed via test run, 1 LEAVE update
- [x] AC-3: All 16 tests in `SearchCoordinator.batching.test.ts` pass (PASS in test output)
- [x] AC-4: Immediate notification path works -- `canDeliver` evaluates to `true` when `this.sendUpdate` is defined, mutations execute, test "should fall back to immediate notification" (line 11) passes
- [x] AC-5: Distributed subscription path works -- `canDeliver` evaluates to `true` when `sub.isDistributed && sub.coordinatorNodeId`, all ClusterSearchCoordinator tests pass (PASS in test output)
- [x] AC-6: Both immediate and batch callbacks simultaneously -- test "should support both immediate and batch callbacks simultaneously" (line 480) passes
- [x] AC-7: Full SearchCoordinator test suite: 4 suites, 67 tests, 0 failures
- [x] Constraint: `computeSubscriptionUpdate()` was NOT modified (verified via git diff -- changes only span lines 796-830)
- [x] Constraint: `SearchSubscription` interface was NOT changed (no additions to interface at line 53)
- [x] Constraint: No new public API (no new exports, no new public methods -- verified via diff)
- [x] Constraint: No spec/bug references in code comments (grep for `sf-043`, `SPEC-043`, `BUG-`, `Phase` returned no matches in source)
- [x] Constraint: Fix is minimal -- exactly 3 guard sites wrapped, 1 `canDeliver` boolean computed, net +11 lines
- [x] Quality: `canDeliver` boolean is computed once per subscription iteration and reused, avoiding redundant evaluation
- [x] Quality: WHY-comment on lines 799-801 explains the reason for the guard clearly without referencing the spec
- [x] Quality: Guard condition `!!(sub.isDistributed && sub.coordinatorNodeId) || !!this.sendUpdate` exactly matches the two delivery paths in the `if (updateType)` block below (lines 830-844)
- [x] Integration: Fits naturally within existing code structure -- same indentation, same style, no new patterns
- [x] Security: No security concerns -- this is a state mutation guard within an internal method
- [x] Architecture: No architectural changes -- single conditional guard within existing method
- [x] Non-duplication: No code duplication introduced
- [x] Cognitive load: Easy to understand -- a developer reading the code can see the guard, read the WHY-comment, and immediately understand the intent

**Summary:** Clean, minimal, and correct bugfix. The `canDeliver` guard at line 802 of `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/search/SearchCoordinator.ts` precisely addresses the root cause by preventing `currentResults` mutations when no delivery path exists. The guard condition mirrors the delivery logic at lines 830-844, making the code self-documenting. All 67 tests pass across 4 suites. All 7 acceptance criteria met. All 5 constraints respected. No issues found.

---

## Completion

**Completed:** 2026-02-09
**Total Commits:** 1
**Audit Cycles:** 1
**Review Cycles:** 1
