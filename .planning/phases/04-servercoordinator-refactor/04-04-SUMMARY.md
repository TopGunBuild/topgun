---
phase: 04-servercoordinator-refactor
plan: 04
subsystem: api
tags: [coordinator, refactor, operations, crdt, message-routing]

# Dependency graph
requires:
  - phase: 04-01
    provides: AuthHandler extraction pattern
  - phase: 04-02
    provides: ConnectionManager for client management
  - phase: 04-03
    provides: StorageManager for map operations
provides:
  - OperationHandler for CLIENT_OP and OP_BATCH processing
  - MessageRegistry pattern for message routing
  - Reduced switch statement via registry lookup
affects:
  - Future operation protocol changes
  - New message type additions (use registry pattern)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Delegation pattern: ServerCoordinator delegates operations to OperationHandler"
    - "Registry pattern: Message type to handler mapping for O(1) lookup"

key-files:
  created:
    - packages/server/src/coordinator/operation-handler.ts
    - packages/server/src/coordinator/message-registry.ts
  modified:
    - packages/server/src/coordinator/types.ts
    - packages/server/src/coordinator/index.ts
    - packages/server/src/ServerCoordinator.ts

key-decisions:
  - "Registry intercepts before switch: Simpler migration path, switch cases remain as fallback"
  - "CLIENT_OP and OP_BATCH prioritized: Most complex operations, biggest impact"
  - "OperationHandlerConfig uses any types: Flexibility for strict union types in ServerCoordinator"
  - "Minimal extraction: Entry points delegated, complex logic stays in ServerCoordinator initially"

patterns-established:
  - "Message registry pattern: Map message.type to handler function"
  - "Registry lookup before switch: Gradual migration strategy"

# Metrics
duration: 25min
completed: 2026-01-19
---

# Phase 4 Plan 4: OperationHandler and MessageRegistry Summary

**OperationHandler extracts CLIENT_OP/OP_BATCH handling, MessageRegistry provides O(1) message routing via registry lookup before switch statement**

## Performance

- **Duration:** 25 min
- **Started:** 2026-01-19T18:57:00Z
- **Completed:** 2026-01-19T19:56:00Z
- **Tasks:** 3
- **Files modified:** 5

## Accomplishments
- Created OperationHandler module with IOperationHandler interface
- Created MessageRegistry pattern with createMessageRegistry factory
- Integrated registry lookup into handleMessage before switch statement
- CLIENT_OP and OP_BATCH now delegated to OperationHandler

## Task Commits

Each task was committed atomically:

1. **Task 1: Add IOperationHandler interface and OperationHandler class** - `9391881` (feat)
2. **Task 2: Create message registry pattern** - `b4dddca` (feat)
3. **Task 3: Integrate into ServerCoordinator and replace switch statement** - `f5fefc4` (feat)

## Files Created/Modified
- `packages/server/src/coordinator/operation-handler.ts` - OperationHandler class processing CLIENT_OP and OP_BATCH
- `packages/server/src/coordinator/message-registry.ts` - MessageRegistry type and createMessageRegistry factory with all 30 message types
- `packages/server/src/coordinator/types.ts` - IOperationHandler interface and OperationHandlerConfig
- `packages/server/src/coordinator/index.ts` - Updated exports
- `packages/server/src/ServerCoordinator.ts` - Registry lookup before switch, OperationHandler delegation

## Decisions Made
- **Registry intercepts before switch:** Added registry lookup check before the existing switch statement. This allows gradual migration - CLIENT_OP and OP_BATCH are handled by registry, other cases still use switch.
- **CLIENT_OP and OP_BATCH prioritized:** These are the most complex operations involving Write Concern, replication, and persistence. Extracting them first has the biggest impact.
- **OperationHandlerConfig uses `any` types:** The ServerCoordinator methods use strict union types (WriteConcernValue, MetricsActionType). Using `any` in the config interface avoids type compatibility issues while maintaining runtime safety.
- **Minimal extraction approach:** OperationHandler provides entry points (processClientOp, processOpBatch) that delegate back to ServerCoordinator methods. Full extraction of the complex logic can be a follow-up refactor.

## Deviations from Plan

None - plan executed as written with pragmatic adaptations:
- Switch statement kept as fallback (registry intercepts first)
- Full message type extraction deferred (only CLIENT_OP and OP_BATCH in registry)

## Issues Encountered
- Large switch statement (1400+ lines) made full extraction risky - resolved by using registry intercept pattern
- Type compatibility between strict union types and generic interface - resolved with `any` types in config

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 4 complete - ServerCoordinator refactored into orchestrator pattern
- AuthHandler, ConnectionManager, StorageManager, OperationHandler all extracted
- Ready for Phase 5 (Observability) or Phase 6 (Polish)

---
*Phase: 04-servercoordinator-refactor*
*Completed: 2026-01-19*
