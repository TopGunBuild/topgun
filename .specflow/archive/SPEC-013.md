---
id: SPEC-013
type: refactor
status: done
priority: high
complexity: small
created: 2026-01-31
---

# Silent Error Handling Audit

## Context

The codebase contains empty catch blocks that silently swallow errors without logging or handling. This makes debugging production issues extremely difficult because errors disappear without trace.

**Audit Findings:**

A comprehensive grep of `catch { }` and `catch (e) { }` patterns found:
- **1 file** with truly problematic silent error swallowing (should be fixed)
- **~25 files** with intentional fallback patterns (acceptable as-is)

The todo referenced:
- `ClusterManager.ts:486` - **FALSE POSITIVE**: This code is inside a multi-line comment block (lines 474-489, commented out with `/* ... */`)
- `client-message-handler.ts:43` - **VALID**: Silent swallowing of HLC parse errors

## Task

Add error logging to the single empty catch block that truly swallows errors without any handling.

## Requirements

### R1. Fix client-message-handler.ts (line 43)

**File:** `packages/server/src/coordinator/client-message-handler.ts`

**Current code (lines 40-44):**
```typescript
} else if (op.orTag) {
    try {
        ts = HLC.parse(op.orTag);
    } catch (e) { }
}
```

**Replace with:**
```typescript
} else if (op.orTag) {
    try {
        ts = HLC.parse(op.orTag);
    } catch (e) {
        logger.debug({ orTag: op.orTag, error: e }, 'Failed to parse HLC from orTag');
    }
}
```

**Rationale:** Use `debug` level because:
- This is a heuristic parse attempt, not a critical error
- Invalid orTag is not unexpected (client may send malformed data)
- Logging at higher level would flood logs during normal operation

## Acceptance Criteria

1. [ ] `client-message-handler.ts:43` logs at debug level when HLC parse fails
2. [ ] Build passes with no TypeScript errors
3. [ ] Existing tests continue to pass

## Constraints

- DO NOT modify catch blocks that have intentional fallback behavior (return values, resolve promises, etc.)
- DO NOT modify catch blocks in test files
- DO NOT change log level of existing loggers
- DO NOT add logging to the commented-out code in ClusterManager.ts
- Use appropriate log levels: `debug` for expected/recoverable, `warn` for operational issues

## Assumptions

- **Logger availability:** Server package has logger utility available (`../utils/logger`)
- **Log level semantics:** Project follows standard log level conventions (debug < info < warn < error)
- **No behavior change:** Adding logging does not change program flow - errors are still caught and handled the same way

---

## Audit History

### Audit v1 (2026-01-31 17:15)
**Status:** NEEDS_REVISION

**Context Estimate:** ~8% total (PEAK range)

**Critical:**

1. **R2 contradicts its own constraint:** The ConflictResolverClient.ts catch blocks at lines 102, 142, and 180 are NOT empty - they already have intentional fallback behavior:
   - Line 102-106: `} catch { this.pendingRequests.delete(requestId); clearTimeout(timeout); resolve({ success: false, error: 'Not connected to server' }); }`
   - Line 142-146: Same pattern with resolve fallback
   - Line 180-184: Resolves with empty array `resolve([])`

   The constraint says "DO NOT modify catch blocks that have intentional fallback behavior" - these catch blocks resolve promises with default values, which IS intentional fallback behavior. R2 should be removed from scope entirely.

2. **R2 method names are incorrect:** Even if R2 were valid:
   - Line 102 is in the `register` method, not "resolveConflict"
   - Line 142 is in the `unregister` method, not "registerResolver"
   - Line 180 is in the `list` method, not "getMergeRejections"

3. **R2 code examples are incomplete:** The "Replace with" code snippets show only partial catch blocks without closing braces or the existing resolve() calls that follow.

4. **AC#2, AC#3, AC#4 are invalid:** Since R2 catch blocks already have fallback behavior and should not be modified per the constraint, these acceptance criteria should be removed.

**Recommendations:**

5. After removing R2, consider re-scoping this spec to only cover R1 (client-message-handler.ts line 43), which IS a truly empty catch block with no fallback behavior.

6. Update the Context section to reflect that only 1 file (not 2) has truly problematic silent error swallowing.

### Response v1 (2026-01-31 17:20)
**Applied:** All critical issues (1-4) and all recommendations (5-6)

**Changes:**
1. [✓] R2 contradicts constraint - Removed entire R2 section (ConflictResolverClient.ts)
2. [✓] R2 method names incorrect - N/A (R2 removed)
3. [✓] R2 code examples incomplete - N/A (R2 removed)
4. [✓] AC#2-4 invalid - Removed AC#2, AC#3, AC#4; kept only AC#1, AC#5, AC#6 (renumbered to AC#1-3)
5. [✓] Re-scope to R1 only - Spec now covers only client-message-handler.ts:43
6. [✓] Update Context section - Changed "2 files" to "1 file" with truly problematic error swallowing

**Rationale:** The ConflictResolverClient.ts catch blocks (lines 102, 142, 180) implement proper error handling by:
- Cleaning up pending requests (delete from map)
- Clearing timeouts to prevent memory leaks
- Resolving promises with fallback values (error messages or empty arrays)

This is intentional fallback behavior that should NOT be modified per the spec's own constraint. The spec now focuses solely on the single truly problematic empty catch block in client-message-handler.ts.

### Audit v2 (2026-01-31 17:45)
**Status:** APPROVED

**Context Estimate:** ~5% total (PEAK range)

**Verification:**

| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Single file, single change, clear rationale |
| Completeness | Pass | Current code, replacement code, and line numbers all verified |
| Testability | Pass | AC#1 verifiable via code inspection; AC#2, AC#3 via build/test |
| Scope | Pass | Minimal scope - one empty catch block |
| Feasibility | Pass | Trivial change - add one log statement |
| Architecture fit | Pass | Uses existing logger already imported at line 14 |
| Non-duplication | Pass | No existing solution being ignored |
| Cognitive load | Pass | Minimal change, easy to understand |
| Strategic fit | Pass | Improves debuggability without behavior change |

**Code Verification:**

- File: `packages/server/src/coordinator/client-message-handler.ts`
- Lines 41-43 confirmed: `try { ts = HLC.parse(op.orTag); } catch (e) { }`
- Logger already imported at line 14: `import { logger } from '../utils/logger';`
- Logger uses pino which supports `logger.debug()` method
- No additional imports needed

**Assumptions Validated:**

- Logger availability: Confirmed - already imported in file
- Log level semantics: Confirmed - pino supports debug level
- No behavior change: Confirmed - error is still caught, only logging added

**Comment:** Clean, focused specification. Single change with clear rationale. All previous issues resolved. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-01-31 17:00
**Commits:** 1

### Files Modified
- `packages/server/src/coordinator/client-message-handler.ts` - Added debug logging to empty catch block at line 43

### Files Created
(none)

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC#1: `client-message-handler.ts:43` logs at debug level when HLC parse fails
- [x] AC#2: Build passes with no TypeScript errors
- [x] AC#3: Existing tests continue to pass (47+ tests verified)

### Deviations
(none)

### Notes
- Logger was already imported at line 14, no additional imports needed
- Debug level chosen per spec rationale (heuristic parse attempt, not critical error)
- Program flow unchanged - error is still caught, only logging added

---

## Review History

### Review v1 (2026-01-31 18:15)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] Exact specification match - Implementation is character-for-character identical to spec requirement R1
- [✓] Logger availability - Logger already imported, no new dependencies
- [✓] Log level appropriate - Debug level correctly used for heuristic parse attempt
- [✓] Build integrity - All packages build successfully with 0 TypeScript errors
- [✓] Test integrity - 47+ tests verified passing
- [✓] No behavior change - Error still caught, only logging added
- [✓] Security - No sensitive data logged, debug level appropriate
- [✓] Code quality - Clean, clear, follows established patterns
- [✓] Architecture fit - Consistent with existing codebase patterns
- [✓] Constraints respected - All 5 constraints followed precisely
- [✓] Commit quality - Proper format, clear message, targeted change

**Summary:** Outstanding implementation of a minimal, focused specification. The single empty catch block at line 43 of `client-message-handler.ts` has been correctly fixed with appropriate debug-level logging. The implementation matches the specification exactly, uses existing logger, follows established patterns, includes proper context, and maintains zero behavior change. All builds pass, tests pass, no regressions.

---

## Completion

**Completed:** 2026-01-31 18:20
**Total Commits:** 1
**Audit Cycles:** 2
**Review Cycles:** 1
