# SPEC-027: Replace Console Statements with Structured Logging

```yaml
id: SPEC-027
type: refactor
status: done
priority: high
complexity: small
created: 2026-02-01
source: TODO-018
```

## Context

TODO-018 identified 2 specific console statements in production code that should use structured logging instead:

1. `packages/core/src/HLC.ts:90` - `console.warn` for clock drift detection
2. `packages/server/src/utils/nativeStats.ts:84-86` - `console.log` for native module status

These are the 2 locations identified in TODO-018 after SPEC-023 standardized all `console.error` calls to pino structured logging. The infrastructure is already in place:
- `packages/core/src/utils/logger.ts` - Core logger (browser-compatible)
- `packages/server/src/utils/logger.ts` - Server logger with pino-pretty in dev

This is a small cleanup task to migrate these 2 specific locations. Note: Additional console statements exist elsewhere in the codebase but are NOT in scope for this specification.

## Task

Replace the 2 specific console statements identified in TODO-018 with pino structured logging to enable consistent log aggregation and debugging for these locations.

## Requirements

### Files to Modify

| File | Line(s) | Current | Change |
|------|---------|---------|--------|
| `packages/core/src/HLC.ts` | 90 | `console.warn(message)` | Import logger, use `logger.warn({ drift, remoteMillis: remote.millis, localMillis: systemTime, maxDriftMs: this.maxDriftMs }, 'Clock drift detected')` |
| `packages/server/src/utils/nativeStats.ts` | 84-88 | 3x `console.log(...)` | Import logger, use `logger.info({ nativeHash: status.nativeHash, sharedArrayBuffer: status.sharedArrayBuffer }, 'Native module status')` |
| `packages/core/src/__tests__/HLC.test.ts` | 350, 434 | `console.warn` spy | Update to spy on `logger.warn` instead |
| `tests/e2e/security/uat-security-hardening.test.ts` | 489, 514, 541 | `console.warn` spy | Update to spy on `logger.warn` instead |

### Change Details

**HLC.ts (line 90):**
```typescript
// Before
console.warn(message);

// After
logger.warn({
  drift,
  remoteMillis: remote.millis,
  localMillis: systemTime,
  maxDriftMs: this.maxDriftMs
}, 'Clock drift detected');
```

Note: The existing `message` variable can be removed since all context is now in the structured fields.

**nativeStats.ts (lines 84-88):**
```typescript
// Before
console.log('[TopGun] Native Module Status:');
console.log(`  - Hash: ${status.nativeHash ? 'native xxHash64' : 'FNV-1a (JS)'}`);
console.log(
  `  - SharedArrayBuffer: ${status.sharedArrayBuffer ? 'available' : 'unavailable'}`
);

// After
logger.info({
  nativeHash: status.nativeHash,
  sharedArrayBuffer: status.sharedArrayBuffer
}, 'Native module status');
```

This consolidates 3 console.log calls into 1 structured log with all data as queryable fields.

**HLC.test.ts (lines 350, 434):**
```typescript
// Before
const warnSpy = jest.spyOn(console, 'warn').mockImplementation();
expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Clock drift detected'));

// After
const warnSpy = jest.spyOn(logger, 'warn').mockImplementation();
expect(warnSpy).toHaveBeenCalledWith(
  expect.objectContaining({ drift: expect.any(Number) }),
  'Clock drift detected'
);
```

**uat-security-hardening.test.ts (lines 489, 514, 541):**
```typescript
// Before
const warnSpy = jest.spyOn(console, 'warn').mockImplementation();
expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Clock drift detected'));

// After
const warnSpy = jest.spyOn(logger, 'warn').mockImplementation();
expect(warnSpy).toHaveBeenCalledWith(
  expect.objectContaining({ drift: expect.any(Number) }),
  'Clock drift detected'
);
```

## Acceptance Criteria

1. **HLC.ts migrated to structured logging**
   - `console.warn` at line 90 replaced with `logger.warn`
   - Structured fields include: drift, remoteMillis, localMillis, maxDriftMs

2. **nativeStats.ts migrated to structured logging**
   - 3x `console.log` at lines 84-88 replaced with single `logger.info`
   - Structured fields include: nativeHash, sharedArrayBuffer

3. **Tests updated to match new logging**
   - HLC.test.ts spies on `logger.warn` instead of `console.warn`
   - uat-security-hardening.test.ts spies on `logger.warn` instead of `console.warn`
   - Test assertions verify structured logging format

4. **Build passes**
   - `pnpm build` completes successfully

5. **Tests pass**
   - `pnpm test` passes (all HLC tests and UAT security tests)

## Constraints

- Use existing loggers (do not create new logger files)
- Core uses `packages/core/src/utils/logger.ts`
- Server uses `packages/server/src/utils/logger.ts`
- Keep same log levels: warn for drift (it's a warning condition), info for status (informational)
- Scope limited to 2 locations from TODO-018 (do not migrate other console statements)

## Assumptions

1. **Log level appropriateness**: `warn` for clock drift is correct since it indicates a potential issue. `info` for native status is correct since it's informational startup output.

2. **Consolidating native status logs**: Combining 3 console.log calls into 1 structured log is preferable for log aggregation.

3. **Tests exist and will be updated**: Multiple tests spy on `console.warn` for HLC drift detection. These tests will be updated to spy on `logger.warn` and verify the structured logging format.

---
*Created: 2026-02-01*

## Audit History

### Audit v1 (2026-02-01 14:22)
**Status:** NEEDS_REVISION

**Context Estimate:** ~10% total

**Critical:**
1. **Assumption 3 is FALSE**: Multiple tests spy on `console.warn` for HLC drift detection and will fail:
   - `packages/core/src/__tests__/HLC.test.ts` lines 350, 434: spy on `console.warn` and assert it was called with "Clock drift detected"
   - `tests/e2e/security/uat-security-hardening.test.ts` lines 489, 514, 541: spy on `console.warn` for drift detection tests

   The constraint "Do NOT modify test files" contradicts the implementation. Either:
   - (A) Remove the constraint and update tests to spy on `logger.warn` instead, OR
   - (B) Keep `console.warn` in HLC.ts and only migrate nativeStats.ts

2. **AC1 and AC2 are overly broad**: The spec claims "zero console.warn/log in production code" but the audit found additional console statements NOT listed in the spec:
   - `packages/core/src/query/adaptive/DefaultIndexingStrategy.ts:120` - `console.warn` for index creation failures
   - `packages/server/src/storage/createStorageAdapter.ts:110` - `console.warn` for deprecation warning
   - `packages/server/src/storage/MemoryServerAdapter.ts:15,20` - `console.log` for storage lifecycle
   - `packages/server/src/storage/BetterSqlite3Adapter.ts:110,155` - `console.log` for SQLite lifecycle
   - `packages/server/src/storage/createStorageAdapter.ts:46,75,85,105` - `console.log` for storage selection

   The AC cannot be satisfied by modifying only the 2 files in scope. Either narrow the AC or expand scope.

**Recommendations:**
3. [Strategic] Consider whether "zero console.warn/log" is the right goal for this spec, or whether the goal should be "migrate the 2 specific locations identified in TODO-018."

### Response v1 (2026-02-01 14:30)
**Applied:** All critical issues and recommendation

**Changes:**
1. Assumption 3 corrected - Updated assumption to acknowledge tests exist and will be updated. Removed "Do NOT modify test files" constraint. Added 2 test files to "Files to Modify" with specific change details.

2. AC1 and AC2 narrowed - Replaced broad "zero console.warn/log in production code" with specific acceptance criteria for the 2 files in scope (HLC.ts and nativeStats.ts). Removed grep-based verification that would fail due to out-of-scope console statements.

3. Recommendation applied - Clarified in Context and Task that this spec migrates "the 2 specific locations identified in TODO-018" not all console statements. Added explicit note that additional console statements exist elsewhere but are NOT in scope. Added constraint to limit scope to TODO-018 locations.

### Audit v2 (2026-02-01 15:45)
**Status:** APPROVED

**Context Estimate:** ~12% total

**Verification:**
| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Task clearly describes migrating 2 specific locations |
| Completeness | Pass | All files listed with exact line numbers and change details |
| Testability | Pass | Each AC is measurable and verifiable |
| Scope | Pass | Explicit constraint limits to TODO-018 locations only |
| Feasibility | Pass | Existing logger infrastructure in place |
| Architecture fit | Pass | Follows SPEC-023 pattern for structured logging |
| Non-duplication | Pass | Uses existing loggers, no new files needed |
| Cognitive load | Pass | Simple 1:1 replacement with clear before/after examples |
| Strategic fit | Pass | Continues logging standardization from SPEC-023 |

**Line Number Verification:**
- HLC.ts line 90: Confirmed `console.warn(message)`
- nativeStats.ts lines 84-88: Confirmed 3x `console.log` (corrected from 84-86)
- HLC.test.ts lines 350, 434: Confirmed `jest.spyOn(console, 'warn')`
- uat-security-hardening.test.ts lines 489, 514, 541: Confirmed spy usage

**Context Breakdown:**
| File | Type | Est. Context |
|------|------|--------------|
| HLC.ts | Simple handler | ~3% |
| nativeStats.ts | Config/utility | ~2% |
| HLC.test.ts | Test file | ~3% |
| uat-security-hardening.test.ts | Test file | ~3% |
| Worker overhead | Fixed | ~5% |
| **Total** | | **~12%** |

**Quality Projection:** PEAK range (0-30%)

**Comment:** Spec is well-formed after v1 revisions. All critical issues from Audit v1 were properly addressed. Line numbers verified accurate (minor correction: nativeStats.ts is lines 84-88, not 84-86, updated in spec). Clear scope boundaries prevent scope creep. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-01 21:40-21:50
**Commits:** 5

### Files Created
None

### Files Modified
- `packages/core/src/HLC.ts` — Replaced console.warn with logger.warn for clock drift detection, added structured fields (drift, remoteMillis, localMillis, maxDriftMs)
- `packages/server/src/utils/nativeStats.ts` — Replaced 3 console.log calls with single logger.info call, added structured fields (nativeHash, sharedArrayBuffer)
- `packages/core/src/__tests__/HLC.test.ts` — Updated 2 tests to spy on logger.warn instead of console.warn, verified structured logging format
- `tests/e2e/security/uat-security-hardening.test.ts` — Updated 3 tests to spy on logger.warn instead of console.warn, verified structured logging format

### Files Deleted
None

### Acceptance Criteria Status
- [x] HLC.ts migrated to structured logging — console.warn replaced with logger.warn, all required structured fields included
- [x] nativeStats.ts migrated to structured logging — 3 console.log calls replaced with 1 logger.info, structured fields included
- [x] Tests updated to match new logging — Both HLC.test.ts and uat-security-hardening.test.ts updated to spy on logger.warn and verify structured format
- [x] Build passes — `pnpm build` completed successfully
- [x] Tests pass — All 31 HLC tests pass, all 24 UAT security tests pass

### Deviations
1. [Rule 1 - Bug] Fixed missed `consoleWarnSpy` reference in HLC.test.ts line 460 — changed to `warnSpy` to match other updates in the same test

### Notes
- Implementation follows the same pattern established in SPEC-023 for structured logging
- All console statements migrated from string concatenation to structured fields for better log aggregation
- Tests now verify structured logging format with `expect.objectContaining({ drift: expect.any(Number) })`
- Committed as 5 atomic commits: 2 production code changes, 2 test updates, 1 fix for missed reference

---

## Review History

### Review v1 (2026-02-01 22:05)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] HLC.ts correctly migrated to structured logging — console.warn replaced with logger.warn at line 90, includes all required structured fields (drift, remoteMillis, localMillis, maxDriftMs)
- [✓] nativeStats.ts correctly migrated to structured logging — 3 console.log calls (lines 84-88) replaced with single logger.info call at line 85, includes all required structured fields (nativeHash, sharedArrayBuffer)
- [✓] Logger imports added correctly — HLC.ts imports from './utils/logger', nativeStats.ts imports from './logger'
- [✓] HLC.test.ts properly updated — Added logger import, spies on logger.warn instead of console.warn at lines 351 and 436, verifies structured format with expect.objectContaining
- [✓] UAT security test properly updated — Added logger import, spies on logger.warn instead of console.warn at lines 490, 518, and 545, verifies structured format
- [✓] Bug fix properly applied — Commit fa7fa9c fixed missed consoleWarnSpy reference at line 460, changed to warnSpy
- [✓] All console.warn/console.log removed from scope files — Verified with grep, no console statements remain in HLC.ts or nativeStats.ts
- [✓] All 31 HLC tests pass — Verified via test run
- [✓] All 24 UAT security tests pass — Verified via test run
- [✓] Build succeeds — pnpm build completed successfully
- [✓] Follows established patterns — Implementation matches SPEC-023 structured logging pattern
- [✓] Commit messages follow project convention — All 5 commits use correct format: refactor(sf-027), test(sf-027), fix(sf-027)
- [✓] No security issues — No hardcoded secrets, proper error handling maintained
- [✓] Code quality high — Clean implementation, structured fields improve observability
- [✓] Scope correctly limited — Only the 2 specific TODO-018 locations modified, no scope creep

**Summary**

Implementation is fully compliant with the specification. All acceptance criteria met. Code quality is excellent with proper structured logging implementation following the SPEC-023 pattern. The bug fix for the missed test spy reference demonstrates thorough testing. No issues found.

The implementation correctly replaces console statements with structured logging for the 2 specific locations identified in TODO-018 (HLC.ts clock drift warning and nativeStats.ts native module status). Tests have been properly updated to spy on logger methods and verify structured format. Build and all tests pass.

---

## Completion

**Completed:** 2026-02-01 22:10
**Total Commits:** 5
**Audit Cycles:** 2
**Review Cycles:** 1
