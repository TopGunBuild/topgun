---
phase: "05"
plan: "03"
subsystem: "client"
tags: ["backpressure", "flow-control", "refactor"]
requires:
  - "05-01"
provides:
  - "backpressure-controller"
  - "flow-control"
affects:
  - "05-04"
tech-stack:
  added: []
  patterns:
    - "controller-extraction"
    - "shared-reference"
key-files:
  created:
    - packages/client/src/sync/BackpressureController.ts
    - packages/client/src/sync/types.ts (IBackpressureController, BackpressureControllerConfig)
  modified:
    - packages/client/src/SyncEngine.ts
    - packages/client/src/sync/index.ts
    - packages/client/src/__tests__/backpressure.test.ts
decisions:
  - id: "shared-oplog-reference"
    decision: "Use shared opLog array reference"
    reason: "BackpressureController needs to count unsynced ops and drop-oldest strategy needs to modify array"
    alternatives: ["callback pattern", "observer pattern"]
  - id: "preserve-array-reference"
    decision: "Fix loadOpLog to mutate array instead of reassigning"
    reason: "Preserves BackpressureController's reference to opLog"
    alternatives: ["lazy initialization", "re-initialize controller after load"]
metrics:
  duration: "15 minutes"
  completed: "2026-01-20"
---

# Phase 05 Plan 03: BackpressureController Extraction Summary

BackpressureController extracted from SyncEngine to manage flow control for pending operations.

## One-liner

BackpressureController extracted with shared opLog reference for pause/throw/drop strategies and water mark events.

## Key Changes

### BackpressureController (packages/client/src/sync/BackpressureController.ts)
- Implements IBackpressureController interface
- Receives shared opLog reference from SyncEngine
- Manages backpressure state: backpressurePaused, waitingForCapacity, highWaterMarkEmitted
- Manages backpressure listeners Map
- Exposes:
  - `getPendingOpsCount()`: Count unsynced ops
  - `getBackpressureStatus()`: Full status object
  - `isBackpressurePaused()`: Pause state
  - `checkBackpressure()`: Pre-operation check (pause/throw/drop)
  - `checkHighWaterMark()`: Post-operation threshold check
  - `checkLowWaterMark()`: Post-ACK threshold check
  - `onBackpressure()`: Event subscription

### SyncEngine Integration
- Added BackpressureController field and initialization
- Delegates public API methods to controller
- Fixed loadOpLog to mutate array (preserves controller reference)
- Updated recordOperation to use controller.checkBackpressure/checkHighWaterMark
- Updated OP_ACK handler to use controller.checkLowWaterMark

### Type Definitions (packages/client/src/sync/types.ts)
- Added IBackpressureController interface
- Added BackpressureControllerConfig type
- Added imports for BackpressureConfig, BackpressureStatus, OpLogEntry

### Test Updates (packages/client/src/__tests__/backpressure.test.ts)
- Updated tests to access checkLowWaterMark via backpressureController
- All 449 tests pass

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| 1 | 22d9ec3 | Add BackpressureController types to sync/types.ts |
| 2 | 5ab4edf | Implement BackpressureController class |
| 3 | 0d32cf9 | Integrate BackpressureController into SyncEngine |

## Architecture Pattern

```
SyncEngine
    |
    +--> backpressureController: BackpressureController
              |
              +--> config: BackpressureConfig
              +--> opLog: OpLogEntry[] (shared reference)
              +--> backpressurePaused, waitingForCapacity, ...
```

The controller receives a reference to SyncEngine's opLog array (not a copy). This allows:
1. Counting unsynced operations via `opLog.filter(op => !op.synced)`
2. Implementing drop-oldest by `opLog.splice(oldestIndex, 1)`

**Critical Fix:** loadOpLog now mutates the existing array instead of reassigning:
```typescript
this.opLog.length = 0;
for (const op of pendingOps) {
  this.opLog.push({ ...op, id: String(op.id), synced: false });
}
```

## Verification

- TypeScript compilation: PASS
- All client tests: 449/450 pass (1 skipped)
- Backpressure tests: All 17 tests pass

## Deviations from Plan

None - plan executed exactly as written.

## Next Phase Readiness

Phase 05-04 (TopicManager extraction) can proceed. BackpressureController is complete and stable.
