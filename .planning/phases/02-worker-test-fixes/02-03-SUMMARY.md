---
phase: 02-worker-test-fixes
plan: 03
subsystem: testing
tags: [jwt, e2e, distributed-search, websocket, authentication]

# Dependency graph
requires:
  - phase: 02-01
    provides: Worker build infrastructure and test:workers script
provides:
  - Working DistributedSearch E2E test with proper JWT authentication
  - BUG-04 (skipped E2E test) resolved
affects: [testing, server, distributed-search]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "JWT auth in E2E tests: Use createTestToken() helper with jwt.sign()"
    - "AUTH_ACK response: Server sends AUTH_ACK not AUTH_SUCCESS/AUTH_RESP"
    - "metricsPort: 0 in tests to avoid port conflicts"

key-files:
  created: []
  modified:
    - packages/server/src/__tests__/DistributedSearch.e2e.test.ts

key-decisions:
  - "Use AUTH_ACK message type for auth response detection (server's actual response)"
  - "Add metricsPort: 0 to avoid port conflicts between test server instances"
  - "Remove deprecated hlc.now() from insertData helper (LWWMap generates timestamp internally)"

patterns-established:
  - "E2E test JWT pattern: jwt.sign with shared secret, createTestToken helper, jwtSecret in server config"
  - "Server config for tests: metricsPort: 0 always to avoid EADDRINUSE errors"

# Metrics
duration: 6min
completed: 2026-01-18
---

# Phase 02 Plan 03: DistributedSearch E2E Test Fix Summary

**E2E distributed search tests unskipped with proper JWT auth, AUTH_ACK handling, and LWWMap API fix - 6 tests passing**

## Performance

- **Duration:** 6 min
- **Started:** 2026-01-18T16:27:48Z
- **Completed:** 2026-01-18T16:33:44Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- Removed describe.skip from DistributedSearch E2E test suite (BUG-04 resolved)
- Added proper JWT authentication infrastructure matching working integration test pattern
- Fixed multiple bugs in test helpers that were causing test failures

## Task Commits

Each task was committed atomically:

1. **Task 1: Add JWT auth infrastructure to DistributedSearch E2E** - `a78f239` (feat)
2. **Task 2: Unskip and run DistributedSearch E2E test** - `4d99777` (fix)

## Files Created/Modified
- `packages/server/src/__tests__/DistributedSearch.e2e.test.ts` - Fixed JWT auth, AUTH_ACK handling, insertData helper, metrics ports

## Decisions Made
- Use AUTH_ACK as the auth success message type (matching server implementation)
- Use metricsPort: 0 to let OS assign random ports and avoid conflicts
- LWWMap.set() signature changed - no longer accepts timestamp as third arg

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed insertData helper using wrong LWWMap.set signature**
- **Found during:** Task 2 (first test run)
- **Issue:** `map.set(key, value, hlc.now())` was passing timestamp where ttlMs is expected
- **Fix:** Changed to `map.set(key, value)` - LWWMap generates timestamp internally via its HLC
- **Files modified:** packages/server/src/__tests__/DistributedSearch.e2e.test.ts
- **Verification:** Test no longer throws "TTL must be a positive finite number"
- **Committed in:** 4d99777 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed AUTH message type detection**
- **Found during:** Task 2 (test timeout debugging)
- **Issue:** Test checked for AUTH_SUCCESS/AUTH_RESP but server sends AUTH_ACK
- **Fix:** Added AUTH_ACK to the message type check
- **Files modified:** packages/server/src/__tests__/DistributedSearch.e2e.test.ts
- **Verification:** Authentication succeeds, search requests are sent
- **Committed in:** 4d99777 (Task 2 commit)

**3. [Rule 1 - Bug] Fixed metrics port conflicts between server instances**
- **Found during:** Task 2 (test run showing EADDRINUSE)
- **Issue:** Multiple ServerCoordinator instances used default port 9090
- **Fix:** Added `metricsPort: 0` to all server configs (random port assignment)
- **Files modified:** packages/server/src/__tests__/DistributedSearch.e2e.test.ts
- **Verification:** No more EADDRINUSE errors, all servers start successfully
- **Committed in:** 4d99777 (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (3 bugs)
**Impact on plan:** All auto-fixes essential for test functionality. The plan described what to add (JWT auth), but didn't account for additional bugs in existing test code that prevented tests from passing. No scope creep.

## Issues Encountered
- Initial test run failed with "TTL must be a positive finite number" - LWWMap API changed since test was written
- Tests timed out waiting for search response - AUTH_ACK mismatch issue
- Metrics port conflict errors - multiple servers binding to port 9090

All issues were resolved as documented in deviations above.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All E2E distributed search tests now passing (6/6)
- BUG-04 requirement satisfied
- Phase 02 (Worker Test Fixes) complete after STATE.md update

---
*Phase: 02-worker-test-fixes*
*Completed: 2026-01-18*
