---
phase: 02-worker-test-fixes
plan: 02
subsystem: testing
tags: [jest, worker-threads, crdt, merkle, serialization, tsup]

# Dependency graph
requires:
  - phase: 02-01
    provides: Worker script compilation via tsup
provides:
  - All worker tests enabled (0 skipped)
  - BUG-01, BUG-02, BUG-03 resolved
  - Worker path resolution for ts-jest environment
  - Unified base.worker.js with all handlers
affects: [02-03, testing, CI]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - require() for bundling side-effect imports in tsup
    - dist path resolution from src directory

key-files:
  created: []
  modified:
    - packages/server/src/__tests__/workers/CRDTMergeWorker.test.ts
    - packages/server/src/__tests__/workers/MerkleWorker.test.ts
    - packages/server/src/__tests__/workers/SerializationWorker.test.ts
    - packages/server/src/workers/WorkerPool.ts
    - packages/server/src/workers/MerkleWorker.ts
    - packages/server/src/workers/SerializationWorker.ts
    - packages/server/src/workers/worker-scripts/base.worker.ts

key-decisions:
  - "base.worker.js bundles all handlers via require() to avoid tree-shaking"
  - "Worker path resolution checks both __dirname and dist/ for compiled workers"
  - "test.worker handlers loaded into base.worker for WorkerPool tests"

patterns-established:
  - "Use require() in worker scripts to force bundling of side-effect modules"
  - "Worker path resolution: check direct path, then dist path, then .ts fallback"

# Metrics
duration: 10min
completed: 2026-01-18
---

# Phase 02 Plan 02: Unskip Worker Thread Tests Summary

**All worker tests unskipped and passing (120 tests, 0 skipped) - resolves BUG-01, BUG-02, BUG-03**

## Performance

- **Duration:** 10 min
- **Started:** 2026-01-18T16:27:44Z
- **Completed:** 2026-01-18T16:37:41Z
- **Tasks:** 3
- **Files modified:** 7

## Accomplishments
- CRDTMergeWorker worker thread tests enabled (BUG-01)
- MerkleWorker worker thread tests enabled (BUG-02)
- SerializationWorker worker thread tests enabled (BUG-03)
- Worker script path resolution fixed for ts-jest environment
- All 120 worker tests pass with 0 skipped

## Task Commits

Each task was committed atomically:

1. **Task 1: Unskip CRDTMergeWorker tests** - `0b4f2b8` (fix)
2. **Task 2: Unskip MerkleWorker tests** - `dac633d` (fix)
3. **Task 3: Unskip SerializationWorker tests** - `285e289` (fix)

**Blocking fix:** `209d27a` (fix: timeout test handler)

## Files Created/Modified
- `packages/server/src/__tests__/workers/CRDTMergeWorker.test.ts` - Unskipped 2 worker thread tests
- `packages/server/src/__tests__/workers/MerkleWorker.test.ts` - Unskipped 3 worker thread tests
- `packages/server/src/__tests__/workers/SerializationWorker.test.ts` - Unskipped describe block (2 tests)
- `packages/server/src/workers/WorkerPool.ts` - Fixed path resolution for dist/ directory
- `packages/server/src/workers/MerkleWorker.ts` - Fixed path resolution for dist/ directory
- `packages/server/src/workers/SerializationWorker.ts` - Fixed path resolution for dist/ directory
- `packages/server/src/workers/worker-scripts/base.worker.ts` - Load all specialized workers via require()

## Decisions Made
- base.worker.js bundles all handlers to support shared WorkerPool pattern
- Worker path resolution tries dist/ path when running via ts-jest
- test.worker handlers also loaded for WorkerPool test coverage

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Worker script path resolution for ts-jest**
- **Found during:** Task 1 (CRDTMergeWorker tests)
- **Issue:** WorkerPool.resolveWorkerScript() used __dirname which points to src/workers in ts-jest, not dist/workers where compiled .js files exist
- **Fix:** Added fallback path resolution: try direct path, then dist path, then .ts fallback
- **Files modified:** WorkerPool.ts, MerkleWorker.ts, SerializationWorker.ts
- **Verification:** Tests run without "Unknown file extension .ts" error
- **Committed in:** 0b4f2b8 (Task 1 commit)

**2. [Rule 3 - Blocking] Base worker missing CRDT/Merkle/Serialization handlers**
- **Found during:** Task 1 (CRDTMergeWorker tests)
- **Issue:** base.worker.js only had handler registry, specialized handlers in separate files weren't loaded
- **Fix:** Added require() calls to load crdt.worker, merkle.worker, serialization.worker into base.worker.ts
- **Files modified:** base.worker.ts
- **Verification:** "Unknown task type" errors resolved, all worker task types work
- **Committed in:** 0b4f2b8 (Task 1 commit)

**3. [Rule 3 - Blocking] Timeout test using non-existent task type**
- **Found during:** Final verification
- **Issue:** WorkerPool.test.ts timeout test used "slow-task" type which doesn't exist in any handler
- **Fix:** Changed test to use "delayed-echo" (existing test.worker handler), added test.worker to base.worker requires
- **Files modified:** WorkerPool.test.ts, base.worker.ts
- **Verification:** All 120 tests pass
- **Committed in:** 209d27a (separate fix commit)

---

**Total deviations:** 3 auto-fixed (3 blocking)
**Impact on plan:** All auto-fixes necessary for tests to function. Worker path resolution was a gap in plan 02-01. No scope creep.

## Issues Encountered
- esbuild tree-shakes bare imports marked as "no side effects" - solved by using require() instead of import
- Interleaved commit from parallel plan 02-03 execution visible in git log (no impact)

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All worker tests now run in CI without skips
- BUG-01, BUG-02, BUG-03 requirements satisfied
- Ready for Plan 02-03: DistributedSearch E2E fix

---
*Phase: 02-worker-test-fixes*
*Completed: 2026-01-18*
