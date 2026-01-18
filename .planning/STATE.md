# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-01-18)

**Core value:** Make TopGun safe for production and easier to maintain
**Current focus:** Phase 4 - ServerCoordinator Refactor

## Current Position

Phase: 4 of 7 (ServerCoordinator Refactor)
Plan: 0 of 4 in current phase
Status: Ready to plan
Last activity: 2026-01-18 - Phase 3 verified and complete

Progress: [==========-----------] 43%

## Performance Metrics

**Velocity:**
- Total plans completed: 9
- Average duration: 5.6 min
- Total execution time: 50 min

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-security-hardening | 3 | 17 min | 5.7 min |
| 02-worker-test-fixes | 3 | 20 min | 6.7 min |
| 03-bug-fixes | 3 | 13 min | 4.3 min |

**Recent Trend:**
- Last 5 plans: 02-02 (10 min), 02-03 (6 min), 03-01 (6 min), 03-02 (4 min), 03-03 (3 min)
- Trend: stable

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- Security first: Production deployments at risk, hardening before refactoring
- Refactor after bugs: Stabilize behavior before restructuring
- LRU over hard limits: Graceful degradation preferred
- 01-01: 10s window, 5 errors max for rate-limited logging (SEC-04)
- 01-01: Only log errorCode, not full error object to prevent log bloat
- 01-02: Separate utility file for JWT validation (clean separation, testable)
- 01-02: Config secret takes precedence over env secret
- 01-02: Error messages include openssl generation hint
- 01-03: Instance maxDriftMs replaces static MAX_DRIFT for configurability
- 01-03: Strict mode throws detailed error with drift value and threshold
- 01-03: Default strictMode=false preserves backwards compatibility
- 02-01: Worker scripts compile to CJS only (required by worker_threads)
- 02-01: Jest warns but does not fail when worker scripts missing
- 02-01: Multi-config tsup pattern for separate build targets
- 02-02: base.worker.js bundles all handlers via require() to avoid tree-shaking
- 02-02: Worker path resolution checks both __dirname and dist/ for compiled workers
- 02-02: test.worker handlers loaded into base.worker for WorkerPool tests
- 02-03: AUTH_ACK is server's auth success message (not AUTH_SUCCESS/AUTH_RESP)
- 02-03: metricsPort: 0 in tests to avoid port conflicts
- 02-03: LWWMap.set() generates timestamp internally (no hlc.now() parameter)
- 03-01: Default waitForReady=true for safe cold start behavior
- 03-01: ensureReady() gates on client.start() for storage initialization
- 03-01: Single readyPromise shared across concurrent requests
- 03-02: Queue topic messages when offline instead of dropping
- 03-02: drop-oldest as default eviction (preserves recent messages)
- 03-02: Flush queue immediately on AUTH_ACK
- 03-03: Check process.env.TOPGUN_DEBUG === 'true' inside method rather than caching as class field
- 03-03: Map size calculations only happen inside debug condition (no wasted CPU when disabled)

### Pending Todos

None yet.

### Blockers/Concerns

Known fragile areas (from CONCERNS.md):
- ClusterMessage type union (30+ message types) - affects Phase 4
- IndexedLWWMap/IndexedORMap complexity - avoid touching during refactors
- ~~Storage loading race conditions - related to BUG-05~~ (FIXED in 03-01)

## Session Continuity

Last session: 2026-01-18
Stopped at: Phase 3 complete, ready for Phase 4
Resume file: None
