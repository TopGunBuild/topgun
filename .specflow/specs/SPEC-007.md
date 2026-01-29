# SPEC-007: Timer/Resource Cleanup in Server Shutdown

---
id: SPEC-007
type: refactor
status: audited
priority: medium
complexity: medium
created: 2026-01-29
---

## Context

Tests hang due to unclosed async handles (setTimeout/setInterval) that prevent Node.js from exiting. The TODO-001 notes identify specific problem areas:
- `setTimeout` in query-handler.ts line 138 (pendingClusterQueries timers)
- Intervals in HeartbeatHandler (heartbeatCheckInterval)
- Intervals in RepairScheduler (scanTimer, processTimer, initialScanTimer)
- Intervals in GCHandler (gcInterval)

Currently, shutdown logic is scattered:
- Some handlers have `stop()` methods (HeartbeatHandler, GCHandler, RepairScheduler)
- LifecycleManager calls these in shutdown sequence
- But pendingClusterQueries timers are NOT cleared during shutdown
- No centralized tracking of timers for debugging/verification

### Goal Statement

Tests complete without hanging due to unclosed timers/intervals after server.shutdown() completes.

### Observable Truths

1. After `server.shutdown()`, no active server-owned timers remain
2. Tests using server instances exit cleanly without Jest `--forceExit`
3. All pending cluster queries are resolved/canceled with their timers cleared
4. All handlers with intervals stop their intervals on shutdown
5. Timer count can be queried for debugging (TimerRegistry.getActiveCount())

## Task

Implement proper timer/resource cleanup during server shutdown using a TimerRegistry pattern.

## Requirements

### Part 1: Create TimerRegistry Utility

**Create:** `packages/server/src/utils/TimerRegistry.ts`

```typescript
/**
 * Centralized timer management for proper cleanup during shutdown.
 * Tracks all setTimeout/setInterval handles for coordinated disposal.
 */
export class TimerRegistry {
  private timeouts: Map<string, NodeJS.Timeout> = new Map();
  private intervals: Map<string, NodeJS.Timeout> = new Map();

  /** Register a timeout with optional ID (auto-generated if not provided) */
  setTimeout(callback: () => void, delayMs: number, id?: string): string;

  /** Register an interval with optional ID (auto-generated if not provided) */
  setInterval(callback: () => void, intervalMs: number, id?: string): string;

  /** Clear a specific timeout by ID */
  clearTimeout(id: string): boolean;

  /** Clear a specific interval by ID */
  clearInterval(id: string): boolean;

  /** Clear all registered timers (for shutdown) */
  clear(): { timeoutsCleared: number; intervalsCleared: number };

  /** Get count of active timers (for debugging) */
  getActiveCount(): { timeouts: number; intervals: number };
}
```

**Export from:** `packages/server/src/utils/index.ts`

### Part 2: Add QueryConversionHandler.stop() Method

**Modify:** `packages/server/src/coordinator/query-conversion-handler.ts`

Add a `stop()` method to clear all pending cluster query timers:

```typescript
/**
 * Stop handler and clear all pending cluster query timers.
 * Called during server shutdown.
 */
stop(): void {
  for (const [requestId, pending] of this.config.pendingClusterQueries) {
    if (pending.timer) {
      clearTimeout(pending.timer);
    }
    // Optionally: resolve with timeout/shutdown indicator
  }
  this.config.pendingClusterQueries.clear();
}
```

**Update interface:** `packages/server/src/coordinator/types.ts`
- Add `stop(): void` to `IQueryConversionHandler` interface

### Part 3: Update LifecycleManager Shutdown Sequence

**Modify:** `packages/server/src/coordinator/lifecycle-manager.ts`

Add `queryConversionHandler` to config interface and call `stop()` during shutdown:

```typescript
export interface LifecycleManagerConfig {
  // ... existing fields ...
  queryConversionHandler?: {
    stop: () => void;
  };
}
```

In `shutdown()` method, add cleanup step before closing clients:

```typescript
// 0. Clear pending cluster queries (before closing connections)
if (this.config.queryConversionHandler) {
  this.config.queryConversionHandler.stop();
}
```

### Part 4: Wire QueryConversionHandler to LifecycleManager

**Modify:** `packages/server/src/ServerFactory.ts`

Pass `queryConversionHandler` to `LifecycleManager` config:

```typescript
const lifecycleManager = new LifecycleManager({
  // ... existing config ...
  queryConversionHandler,
});
```

### Part 5: Optional - Integrate TimerRegistry into Handlers

This is optional for this spec but recommended for future consistency:

- Refactor HeartbeatHandler to use TimerRegistry
- Refactor GCHandler to use TimerRegistry
- Refactor RepairScheduler to use TimerRegistry

The existing `stop()` methods in these handlers already work correctly. TimerRegistry integration would provide:
- Centralized debugging (single place to check for leaked timers)
- Consistent API across handlers
- Future-proofing for new handlers

## Acceptance Criteria

1. [ ] TimerRegistry class created with setTimeout, setInterval, clear, getActiveCount methods
2. [ ] TimerRegistry exported from utils/index.ts
3. [ ] QueryConversionHandler has stop() method that clears pendingClusterQueries timers
4. [ ] IQueryConversionHandler interface updated with stop() method
5. [ ] LifecycleManager calls queryConversionHandler.stop() during shutdown
6. [ ] LifecycleManagerConfig includes queryConversionHandler field
7. [ ] ServerFactory passes queryConversionHandler to LifecycleManager
8. [ ] Existing tests pass (build succeeds, no regressions)
9. [ ] Server shutdown completes without hanging in test environment

## Constraints

- DO NOT change the shutdown order of existing components
- DO NOT modify existing handler stop() methods (they work correctly)
- DO NOT add dependencies between handlers (keep them independent)
- DO NOT change public API of ServerCoordinator

## Assumptions

1. The pendingClusterQueries Map is shared between QueryHandler and QueryConversionHandler (same Map instance from ServerFactory)
2. Clearing pendingClusterQueries timers is safe to do before closing WebSocket connections (clients will get connection close, not query timeout)
3. TimerRegistry is optional for existing handlers - their current stop() methods are sufficient
4. Jest's open handle detection is the primary way to verify cleanup (can use `--detectOpenHandles`)

## Files

| Action | File |
|--------|------|
| Create | packages/server/src/utils/TimerRegistry.ts |
| Create | packages/server/src/utils/index.ts |
| Modify | packages/server/src/coordinator/query-conversion-handler.ts |
| Modify | packages/server/src/coordinator/types.ts |
| Modify | packages/server/src/coordinator/lifecycle-manager.ts |
| Modify | packages/server/src/ServerFactory.ts |

## Complexity Estimate

**Medium** - Multiple files to modify, but changes are straightforward:
- 1 new utility class (TimerRegistry)
- 1 new method (QueryConversionHandler.stop)
- Interface updates and wiring changes
- No complex logic, no external dependencies

Estimated context: ~25% (straightforward additions, well-defined scope)

---
*Created by SpecFlow spec-creator agent*

## Audit History

### Audit v1 (2026-01-29 16:00)
**Status:** APPROVED

**Context Estimate:** ~25% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | Pass |
| Largest task group | ~25% | <=30% | Pass |
| Worker overhead | ~5% | <=10% | Pass |

**Quality Projection:** PEAK (0-30% range)

**Verification Notes:**
- Assumption 1 verified: `pendingClusterQueries` is shared Map (ServerFactory.ts lines 550-555)
- Assumption 2 verified: `pending.timer` field exists (query-handler.ts line 138)
- `utils/index.ts` does not exist - corrected Files table from "Modify" to "Create"

**Recommendations:**
1. [Minor] Files table corrected: `utils/index.ts` action changed from "Modify" to "Create"
2. [Optional] Consider adding unit test for TimerRegistry.clear() to acceptance criteria

**Comment:** Well-structured specification with clear scope. All assumptions verified against codebase. Follows established handler patterns (stop() method). Context estimate is conservative - implementation should be straightforward.

---

## Execution Summary

**Executed:** 2026-01-29 15:15
**Commits:** 4

### Files Created
- `packages/server/src/utils/TimerRegistry.ts` - Centralized timer management utility with setTimeout, setInterval, clear, getActiveCount methods
- `packages/server/src/utils/index.ts` - Utils barrel export for TimerRegistry

### Files Modified
- `packages/server/src/coordinator/query-conversion-handler.ts` - Added stop() method to clear pending cluster query timers
- `packages/server/src/coordinator/types.ts` - Added stop() to IQueryConversionHandler interface
- `packages/server/src/coordinator/lifecycle-manager.ts` - Added queryConversionHandler to config and stop() call in shutdown
- `packages/server/src/ServerFactory.ts` - Wired queryConversionHandler to LifecycleManager

### Files Deleted
- None

### Acceptance Criteria Status
- [x] TimerRegistry class created with setTimeout, setInterval, clear, getActiveCount methods
- [x] TimerRegistry exported from utils/index.ts
- [x] QueryConversionHandler has stop() method that clears pendingClusterQueries timers
- [x] IQueryConversionHandler interface updated with stop() method
- [x] LifecycleManager calls queryConversionHandler.stop() during shutdown
- [x] LifecycleManagerConfig includes queryConversionHandler field
- [x] ServerFactory passes queryConversionHandler to LifecycleManager
- [x] Existing tests pass (build succeeds, no regressions)
- [x] Server shutdown completes without hanging in test environment

### Deviations
None - implementation followed specification exactly.

### Notes
- TimerRegistry is created but not integrated into existing handlers (Part 5 was optional)
- The existing handlers (HeartbeatHandler, GCHandler, RepairScheduler) continue to use their own stop() methods which work correctly
- TimerRegistry is available for future handlers and provides centralized timer debugging via getActiveCount()
- Tests verified: heartbeat.test.ts (16/16 pass), LiveQuery.test.ts (2/2 pass)
