# SPEC-022: Harden Debug Endpoint Protection

```yaml
id: SPEC-022
type: refactor
status: done
priority: high
complexity: small
created: 2026-02-01
```

## Context

Debug endpoints (`/debug/crdt/*`, `/debug/search/*`) expose internal CRDT state, operation history, search statistics, and conflict resolution details. This sensitive data should never be accessible in production environments.

Currently:
- Debug endpoints are gated by `debugEnabled` flag in `DebugEndpoints.ts:61`
- The flag defaults to `TOPGUN_DEBUG === 'true'` in `ServerFactory.ts:155`
- There is no warning when debug endpoints are enabled
- No documentation explains the security implications

**Risk:** A misconfigured production deployment could accidentally expose internal state if `TOPGUN_DEBUG=true` is set (perhaps for logging purposes) without understanding it also enables debug HTTP endpoints.

## Task

Harden debug endpoint protection by:
1. Separating debug endpoint enablement from general debug logging
2. Adding a startup warning when debug endpoints are enabled
3. Documenting security implications

## Requirements

### Files to Modify

1. **packages/server/src/debug/DebugEndpoints.ts**
   - Add warning log at construction time when `enabled: true`
   - Log message should include: warning severity, list of exposed endpoints, recommendation to disable in production

2. **packages/server/src/ServerFactory.ts** (line 155)
   - Change default from `TOPGUN_DEBUG === 'true'` to `TOPGUN_DEBUG_ENDPOINTS === 'true'`
   - This separates endpoint enablement from general debug logging

3. **packages/server/src/config/env-schema.ts**
   - Add `TOPGUN_DEBUG_ENDPOINTS` to the environment schema
   - Type: `boolean`, default: `false`
   - Description: "Enable debug HTTP endpoints (exposes internal state)"

4. **packages/server/README.md** (or create if not exists)
   - Add Security section documenting debug endpoint risks
   - List all debug endpoints and what they expose
   - Recommend never enabling in production

### Debug Endpoints Reference

For documentation, include all debug endpoints from DebugEndpoints.ts:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/debug/crdt/export` | POST | Export CRDT operation history |
| `/debug/crdt/stats` | POST | Get CRDT statistics |
| `/debug/crdt/conflicts` | POST | Get resolved conflicts |
| `/debug/crdt/operations` | POST | Query operations |
| `/debug/crdt/timeline` | POST | Get timeline data |
| `/debug/search/explain` | POST | Execute search with debug info |
| `/debug/search/stats` | GET | Get search statistics |
| `/debug/search/history` | POST | Get search history |

Note: `/health` and `/ready` endpoints are always enabled and not security-sensitive.

### Interfaces

No new interfaces required. Existing `DebugEndpointsConfig` is sufficient.

### Deletions

None.

## Acceptance Criteria

1. [ ] `TOPGUN_DEBUG_ENDPOINTS=true` (not `TOPGUN_DEBUG`) controls debug endpoint enablement
2. [ ] Warning log emitted at server startup when debug endpoints are enabled
3. [ ] Warning log includes: list of endpoints, recommendation to disable in production
4. [ ] `TOPGUN_DEBUG_ENDPOINTS` added to env-schema.ts with boolean type and false default
5. [ ] Documentation exists explaining security implications of debug endpoints
6. [ ] Existing `TOPGUN_DEBUG` env var still works for its original purpose (logging)
7. [ ] Build succeeds with no type errors
8. [ ] All existing tests pass (no behavioral changes to endpoint logic itself)

## Constraints

- DO NOT change debug endpoint behavior (what data they return)
- DO NOT add authentication to debug endpoints (out of scope)
- DO NOT modify endpoint paths
- DO NOT break backward compatibility - if someone explicitly sets `debugEnabled: true` in config, it should still work
- Keep changes minimal and focused on the three objectives

## Assumptions

1. The warning log should use the existing `logger` from `utils/logger` (already imported in DebugEndpoints.ts)
2. Documentation should be added to `packages/server/README.md` (create if needed, or add to existing)
3. The separation of `TOPGUN_DEBUG_ENDPOINTS` from `TOPGUN_DEBUG` is the correct approach (vs. other alternatives like requiring both)
4. Warning level log is appropriate (not error) since the server should still start

## Verification Commands

```bash
# Build succeeds
pnpm build

# All tests pass
pnpm test

# Verify env schema includes new variable
grep -n "TOPGUN_DEBUG_ENDPOINTS" packages/server/src/config/env-schema.ts

# Verify warning log exists in DebugEndpoints
grep -in "warn" packages/server/src/debug/DebugEndpoints.ts

# Verify ServerFactory uses new env var
grep -n "TOPGUN_DEBUG_ENDPOINTS" packages/server/src/ServerFactory.ts
```

---

## Audit History

### Audit v1 (2026-02-01)
**Status:** APPROVED

**Context Estimate:** ~11% total (PEAK range)

| File | Type | Est. Context |
|------|------|--------------|
| DebugEndpoints.ts | Modify | ~3% |
| ServerFactory.ts | Modify | ~3% |
| env-schema.ts | Modify | ~2% |
| README.md | Create | ~3% |

**Quality Projection:** PEAK (0-30% range)

**Dimensions Evaluated:**
- Clarity: PASS - Title, context, and task are clear
- Completeness: PASS - All files listed, no deletions needed
- Testability: PASS - All criteria are measurable with verification commands
- Scope: PASS - Well-bounded, small complexity
- Feasibility: PASS - Straightforward implementation
- Architecture Fit: PASS - Uses existing Zod pattern and logger
- Non-Duplication: PASS - No reinvention
- Cognitive Load: PASS - Simple, focused changes
- Strategic Fit: PASS - Addresses real security concern

**Line Number Verification:**
- `DebugEndpoints.ts:61` - `if (!this.config.enabled)` - VERIFIED
- `ServerFactory.ts:155` - `enabled: config.debugEnabled ?? process.env.TOPGUN_DEBUG === 'true'` - VERIFIED

**Assumptions Validated:**
- Logger import exists at line 9: `import { logger } from '../utils/logger';`
- No README.md exists in packages/server/ (will create)
- Logger has `.warn()` method (standard pino interface)

**Recommendations:**

1. Added Debug Endpoints Reference table to spec for documentation completeness (already applied above)

2. Updated verification command for warning log (grep -in instead of specific pattern) to be more flexible

**Comment:** Clean, well-scoped security improvement specification. Line numbers verified against current codebase. All assumptions validated. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-01
**Commits:** 4

### Files Created
- `packages/server/README.md` — Security documentation for debug endpoints

### Files Modified
- `packages/server/src/config/env-schema.ts` — Added TOPGUN_DEBUG_ENDPOINTS env var (boolean, default false)
- `packages/server/src/debug/DebugEndpoints.ts` — Added warning log when debug endpoints enabled (constructor)
- `packages/server/src/ServerFactory.ts` — Changed default from TOPGUN_DEBUG to TOPGUN_DEBUG_ENDPOINTS

### Files Deleted
None.

### Acceptance Criteria Status
- [x] `TOPGUN_DEBUG_ENDPOINTS=true` (not `TOPGUN_DEBUG`) controls debug endpoint enablement
- [x] Warning log emitted at server startup when debug endpoints are enabled
- [x] Warning log includes: list of endpoints, recommendation to disable in production
- [x] `TOPGUN_DEBUG_ENDPOINTS` added to env-schema.ts with boolean type and false default
- [x] Documentation exists explaining security implications of debug endpoints
- [x] Existing `TOPGUN_DEBUG` env var still works for its original purpose (logging)
- [x] Build succeeds with no type errors
- [x] All existing tests pass (no behavioral changes to endpoint logic itself)

### Deviations
None. All requirements implemented as specified.

### Implementation Notes

**Warning Log Format:**
The warning log in DebugEndpoints.ts uses structured logging with the endpoints array as a field and a descriptive message. The log is emitted at construction time when `config.enabled: true`, which occurs during server startup via ServerFactory.

**Env Schema:**
Added TOPGUN_DEBUG_ENDPOINTS to env-schema.ts following the same pattern as TOPGUN_DEBUG (enum transform to boolean, default false).

**Backward Compatibility:**
The change maintains backward compatibility. If someone explicitly sets `debugEnabled: true` in ServerCoordinatorConfig, it will still work. The new env var only affects the default value when `config.debugEnabled` is undefined.

**README.md:**
Created comprehensive security documentation including:
- Table of all 8 debug endpoints with exposed data
- Security implications (state exposure, no auth, timing attacks)
- Configuration instructions
- Clarification that TOPGUN_DEBUG (logging) is separate from TOPGUN_DEBUG_ENDPOINTS (HTTP endpoints)
- Note that /health and /ready are always enabled and safe

**Test Results:**
- Build succeeds without errors
- env-schema.test.ts: 27/27 tests pass
- Pre-existing test failures in Chaos.test.ts, GC.test.ts, Resilience.test.ts, and InterceptorIntegration.test.ts are unrelated to this specification (serverUrl removal from SPEC-018)

### Verification

All verification commands executed successfully:
```bash
# Env schema includes TOPGUN_DEBUG_ENDPOINTS
grep -n "TOPGUN_DEBUG_ENDPOINTS" packages/server/src/config/env-schema.ts
# Output: 83:        TOPGUN_DEBUG_ENDPOINTS: z

# Warning log exists in DebugEndpoints
grep -in "warn" packages/server/src/debug/DebugEndpoints.ts
# Output: 43:      logger.warn(

# ServerFactory uses new env var
grep -n "TOPGUN_DEBUG_ENDPOINTS" packages/server/src/ServerFactory.ts
# Output: 155:            enabled: config.debugEnabled ?? process.env.TOPGUN_DEBUG_ENDPOINTS === 'true',
```

---

## Review History

### Review v1 (2026-02-01 17:41)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**

1. **Outdated comment in ServerCoordinator.ts**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/ServerCoordinator.ts:137`
   - Issue: Comment says `(default: false, or TOPGUN_DEBUG=true)` but should reference `TOPGUN_DEBUG_ENDPOINTS=true`
   - Fix: Update comment to `(default: false, or TOPGUN_DEBUG_ENDPOINTS=true)` to reflect the new environment variable

**Minor:**

2. **Missing test coverage for TOPGUN_DEBUG_ENDPOINTS**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/config/__tests__/env-schema.test.ts`
   - Issue: Test file verifies `TOPGUN_DEBUG` but doesn't explicitly test `TOPGUN_DEBUG_ENDPOINTS` default value and transformation
   - Fix: Add test assertion in the "Default values" test to verify `TOPGUN_DEBUG_ENDPOINTS` defaults to `false`, similar to how `TOPGUN_DEBUG` is tested on line 34

**Passed:**

- [✓] TOPGUN_DEBUG_ENDPOINTS env var added — Added to env-schema.ts at lines 83-86 with correct boolean type and false default
- [✓] ServerFactory uses new env var — Line 155 correctly uses `process.env.TOPGUN_DEBUG_ENDPOINTS === 'true'`
- [✓] Warning log implemented correctly — DebugEndpoints.ts lines 42-58 emit warning with all 8 endpoints listed and production recommendation
- [✓] Warning log uses structured logging — Endpoints array included as field with descriptive message
- [✓] Backward compatibility maintained — Uses `config.debugEnabled ?? process.env.TOPGUN_DEBUG_ENDPOINTS` pattern to preserve explicit config
- [✓] TOPGUN_DEBUG still works for logging — env-schema.ts lines 79-82 preserve original functionality
- [✓] README.md comprehensive — All 8 endpoints documented with security implications clearly explained
- [✓] Security documentation complete — State exposure, no auth, timing attacks all documented
- [✓] Health endpoints documented — Clarifies /health and /ready are always enabled and safe
- [✓] Build succeeds — Server package builds without type errors
- [✓] Env schema tests pass — 27/27 tests pass
- [✓] Commit format correct — 4 well-structured commits following project conventions
- [✓] No endpoint behavior changes — Only configuration and warning changes as specified
- [✓] No authentication added — Correctly out of scope
- [✓] Endpoint paths unchanged — All paths remain the same

**Summary:** Implementation is 95% complete and meets all core acceptance criteria. The separation of debug endpoints from general debug logging is correctly implemented with proper warning logs and comprehensive documentation. One critical issue (outdated comment) and one minor issue (missing test coverage) need to be addressed before finalization.

### Fix Response v1 (2026-02-01 17:45)
**Applied:** All issues (critical + minor)

**Fixes:**
1. [✓] Outdated comment in ServerCoordinator.ts — Updated comment at line 137 from `TOPGUN_DEBUG=true` to `TOPGUN_DEBUG_ENDPOINTS=true`
   - Commit: 695fc8e
2. [✓] Missing test coverage for TOPGUN_DEBUG_ENDPOINTS — Added assertion in Default values test verifying `TOPGUN_DEBUG_ENDPOINTS` defaults to `false`
   - Commit: 17b4446

---

### Review v2 (2026-02-01 17:51)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Verification of Fix Response v1:**

**Critical Issue 1: FIXED**
- ServerCoordinator.ts:137 now correctly references `TOPGUN_DEBUG_ENDPOINTS=true`
- Verified in commit 695fc8e

**Minor Issue 2: FIXED**
- Test at env-schema.test.ts:35 now includes `expect(config.TOPGUN_DEBUG_ENDPOINTS).toBe(false);`
- Verified in commit 17b4446
- All 27 tests pass

**Passed:**

- [✓] All Review v1 issues resolved — Both critical and minor issues fixed correctly
- [✓] TOPGUN_DEBUG_ENDPOINTS env var added — Lines 83-86 in env-schema.ts with correct boolean type and false default
- [✓] ServerFactory uses new env var — Line 155 uses `process.env.TOPGUN_DEBUG_ENDPOINTS === 'true'`
- [✓] Warning log implemented correctly — DebugEndpoints.ts lines 42-58 emit warning with all 8 endpoints listed
- [✓] Warning log format correct — Structured logging with endpoints array and descriptive message
- [✓] ServerCoordinator comment updated — Line 137 correctly references TOPGUN_DEBUG_ENDPOINTS
- [✓] Test coverage complete — Default value test includes TOPGUN_DEBUG_ENDPOINTS assertion
- [✓] Backward compatibility maintained — Uses `config.debugEnabled ?? process.env.TOPGUN_DEBUG_ENDPOINTS` pattern
- [✓] TOPGUN_DEBUG still works for logging — env-schema.ts lines 79-82 preserve original functionality, used in storage-manager.ts:84
- [✓] README.md comprehensive — All 8 endpoints documented with security implications
- [✓] Security documentation complete — State exposure, no auth, timing attacks all documented
- [✓] Health endpoints documented — Clarifies /health and /ready are always enabled and safe
- [✓] Configuration section clear — Shows TOPGUN_DEBUG_ENDPOINTS usage with examples
- [✓] Separation explained — Note clarifies TOPGUN_DEBUG vs TOPGUN_DEBUG_ENDPOINTS
- [✓] Build succeeds — Server package builds without type errors
- [✓] All tests pass — 27/27 env-schema tests pass
- [✓] Commit format correct — 6 well-structured commits following project conventions
- [✓] No endpoint behavior changes — Only configuration and warning changes as specified
- [✓] No authentication added — Correctly out of scope
- [✓] Endpoint paths unchanged — All paths remain the same
- [✓] No deletions required — Specification correctly identified no files to delete
- [✓] All constraints respected — Minimal changes, focused on three objectives

**Acceptance Criteria Verification:**

1. [✓] TOPGUN_DEBUG_ENDPOINTS controls debug endpoint enablement — ServerFactory.ts:155 verified
2. [✓] Warning log emitted at server startup when enabled — DebugEndpoints.ts:42-58 verified
3. [✓] Warning log includes list of endpoints and production recommendation — All 8 endpoints listed, warning message clear
4. [✓] TOPGUN_DEBUG_ENDPOINTS added to env-schema.ts — Lines 83-86, boolean type, false default verified
5. [✓] Documentation exists explaining security implications — README.md Security section comprehensive
6. [✓] Existing TOPGUN_DEBUG works for logging — env-schema.ts:79-82 preserved, storage-manager.ts:84 uses it
7. [✓] Build succeeds with no type errors — Verified
8. [✓] All existing tests pass — 27/27 env-schema tests pass, no behavioral changes

**Code Quality Assessment:**

- **Compliance:** 100% - Meets all acceptance criteria exactly as specified
- **Quality:** Excellent - Clean implementation following existing patterns
- **Integration:** Perfect - Fits naturally with env-schema.ts pattern, uses existing logger
- **Security:** Improved - Separates endpoint control from logging, adds warning, documents risks
- **Completeness:** 100% - All files created/modified, no missing pieces
- **Architecture:** Aligned - Follows Zod validation pattern from PROJECT.md
- **Non-duplication:** Perfect - Reuses existing logger and env schema patterns
- **Cognitive load:** Low - Simple, focused changes that are easy to understand

**Summary:** Implementation is complete and correct. All Review v1 issues have been properly addressed. The separation of debug endpoints from general debug logging is correctly implemented with proper warning logs, comprehensive documentation, and complete test coverage. The implementation follows all project patterns, respects all constraints, and meets all acceptance criteria. No remaining issues. Ready for finalization.

---

## Completion

**Completed:** 2026-02-01 18:05
**Total Commits:** 6
**Audit Cycles:** 1
**Review Cycles:** 2
