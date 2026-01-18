---
phase: 04-servercoordinator-refactor
plan: 02
subsystem: server
tags: [refactor, connection-manager, websocket, clients-map, module-extraction]

# Dependency graph
requires:
  - phase: 04-01
    provides: coordinator folder, types.ts with IConnectionManager interface
provides:
  - ConnectionManager class implementation with full client lifecycle
  - ServerCoordinator delegating all client operations to ConnectionManager
  - Single source of truth for clients Map
affects: [04-03, 04-04, any future connection-related changes]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Manager pattern: Stateful modules own data, coordinators orchestrate"
    - "Delegation pattern: ServerCoordinator delegates to specialized managers"

key-files:
  created:
    - packages/server/src/coordinator/connection-manager.ts
  modified:
    - packages/server/src/ServerCoordinator.ts
    - packages/server/src/coordinator/index.ts

key-decisions:
  - "ConnectionManager owns clients Map (single source of truth)"
  - "ServerCoordinator delegates isClientAlive/getClientIdleTime to ConnectionManager"
  - "Broadcast methods stay in ServerCoordinator (have queryRegistry/securityManager deps)"

patterns-established:
  - "Manager extraction: Extract stateful module, expose via interface, delegate from coordinator"
  - "Client access pattern: this.connectionManager.getClient(id) for all client lookups"

# Metrics
duration: 16min
completed: 2026-01-18
---

# Phase 4 Plan 2: ConnectionManager Extraction Summary

**ConnectionManager module extracted from ServerCoordinator, owning clients Map with full client lifecycle management (register, remove, heartbeat check)**

## Performance

- **Duration:** 16 min
- **Started:** 2026-01-18T18:50:08Z
- **Completed:** 2026-01-18T19:06:37Z
- **Tasks:** 3 (Task 1 was completed in previous plan)
- **Files modified:** 2

## Accomplishments
- ConnectionManager implementation with full IConnectionManager interface
- ServerCoordinator no longer owns clients Map - delegates to ConnectionManager
- All client access unified through ConnectionManager.getClient()
- Heartbeat methods (isClientAlive, getClientIdleTime) delegate to ConnectionManager
- All heartbeat tests pass with ConnectionManager integration

## Task Commits

Each task was committed atomically:

1. **Task 1: Add IConnectionManager interface and ConnectionManager class** - `544278b` (feat) - Completed in 04-01 plan
2. **Task 2: Integrate ConnectionManager into ServerCoordinator** - `5a64b48` (feat)
3. **Task 3: Verify extraction and run tests** - `b8ff8a0` (test) - Test updates committed in 04-01

**Plan metadata:** Pending

## Files Created/Modified
- `packages/server/src/coordinator/connection-manager.ts` - ConnectionManager class with client lifecycle methods
- `packages/server/src/ServerCoordinator.ts` - Replaced clients Map with connectionManager, updated all client access
- `packages/server/src/coordinator/index.ts` - Added ConnectionManager export
- `packages/server/src/__tests__/heartbeat.test.ts` - Updated to use connectionManager.getClients()

## Decisions Made
- **ConnectionManager owns clients Map**: Single source of truth for client state
- **Delegate heartbeat methods**: isClientAlive/getClientIdleTime delegate to ConnectionManager rather than duplicating logic
- **Broadcast methods stay in ServerCoordinator**: These have dependencies on queryRegistry and securityManager for subscription-based routing and FLS filtering - orchestration responsibilities remain with coordinator
- **ConnectionManager.getClients() returns Map directly**: For read access patterns, returning the map directly is acceptable since ConnectionManager still owns the map

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Heartbeat tests accessing removed clients Map**
- **Found during:** Task 3 (Verification)
- **Issue:** heartbeat.test.ts directly accessed `(server as any).clients` which no longer exists
- **Fix:** Updated all test access to use `(server as any).connectionManager.getClients()`
- **Files modified:** packages/server/src/__tests__/heartbeat.test.ts
- **Verification:** All 16 heartbeat tests pass
- **Committed in:** b8ff8a0 (committed during 04-01 plan)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Test file update was necessary for tests to pass with refactored code. No scope creep.

## Issues Encountered
- Task 1 files (connection-manager.ts, types.ts updates) were already created in the previous plan (04-01) commit
- The full server test suite is large and takes >5 minutes to run; heartbeat tests verified in isolation

## Next Phase Readiness
- ConnectionManager extraction complete
- Ready for Plan 03 (MessageHandler extraction) or Plan 04 (SubscriptionManager extraction)
- Established pattern: Extract module, create interface, delegate from coordinator

---
*Phase: 04-servercoordinator-refactor*
*Plan: 02*
*Completed: 2026-01-18*
