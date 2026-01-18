---
phase: 02-worker-test-fixes
plan: 01
subsystem: infra
tags: [tsup, jest, worker-threads, build, testing]

# Dependency graph
requires:
  - phase: 01-security-hardening
    provides: Stable codebase ready for test infrastructure improvements
provides:
  - Worker script compilation via tsup build
  - test:workers script for running worker tests with pre-build
  - Jest setup warning when worker scripts not compiled
affects: [02-02, 02-03, testing, workers]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - Multi-entry tsup config (array of configs)
    - Jest setupFilesAfterEnv for build validation

key-files:
  created:
    - packages/server/jest.setup.js
  modified:
    - packages/server/tsup.config.ts
    - packages/server/package.json
    - packages/server/jest.config.js
    - .gitignore

key-decisions:
  - "Worker scripts compile to CJS only (required by worker_threads)"
  - "Worker scripts output to dist/workers/worker-scripts/ preserving structure"
  - "Jest warns but does not fail when worker scripts missing (non-blocking)"

patterns-established:
  - "Multi-config tsup: Use array of defineConfig for separate build targets"
  - "Build-then-test: Use pnpm build && jest for tests requiring compiled artifacts"

# Metrics
duration: 4min
completed: 2026-01-18
---

# Phase 02 Plan 01: Worker Build Infrastructure Summary

**tsup config compiles worker scripts to JS, test:workers script builds first, Jest warns if workers missing**

## Performance

- **Duration:** 4 min
- **Started:** 2026-01-18T16:22:35Z
- **Completed:** 2026-01-18T16:26:09Z
- **Tasks:** 3
- **Files modified:** 5

## Accomplishments
- Worker scripts now compile to dist/workers/worker-scripts/*.js during build
- New test:workers script ensures workers are compiled before tests run
- Jest displays warning when attempting to run tests without compiled workers

## Task Commits

Each task was committed atomically:

1. **Task 1: Update tsup config to compile worker scripts** - `1e000d5` (feat)
2. **Task 2: Add test:workers script with build step** - `299f8aa` (feat)
3. **Task 3: Update Jest globalSetup to ensure build before worker tests** - `9b26187` (feat)

## Files Created/Modified
- `packages/server/tsup.config.ts` - Added worker scripts entry points as second config
- `packages/server/package.json` - Added test:workers script
- `packages/server/jest.config.js` - Added setupFilesAfterEnv
- `packages/server/jest.setup.js` - Warns if worker scripts not compiled
- `.gitignore` - Added exception for jest.setup.js files

## Decisions Made
- Worker scripts compile to CJS only (worker_threads requires .js files)
- Output directory preserves worker-scripts structure for WorkerPool.resolveWorkerScript()
- Jest warning is non-fatal since inline worker tests work without compilation

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added .gitignore exception for jest.setup.js**
- **Found during:** Task 3 (Jest setup creation)
- **Issue:** The root .gitignore ignores all *.js files, preventing jest.setup.js from being committed
- **Fix:** Added `!packages/**/jest.setup.js` exception to .gitignore
- **Files modified:** .gitignore
- **Verification:** git add succeeded after adding exception
- **Committed in:** 9b26187 (Task 3 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Auto-fix necessary to commit the new file. No scope creep.

## Issues Encountered
- Worker thread errors still appear in test output when WorkerPool resolves to src/*.ts files instead of dist/*.js - this is expected behavior during ts-jest execution and will be addressed in subsequent plans

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Build infrastructure complete for worker script compilation
- Ready for Plan 02-02: WorkerPool path resolution to prefer .js files
- Ready for Plan 02-03: Remove skipped worker tests

---
*Phase: 02-worker-test-fixes*
*Completed: 2026-01-18*
