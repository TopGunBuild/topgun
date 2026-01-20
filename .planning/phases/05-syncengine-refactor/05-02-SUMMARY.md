---
phase: 05
plan: 02
name: "QueryManager Extraction"
subsystem: client-sync

dependency-graph:
  requires:
    - "05-01 (WebSocketManager)"
  provides:
    - "QueryManager class"
    - "IQueryManager interface"
    - "Query subscription management"
    - "Local query execution"
  affects:
    - "05-03-PLAN (completed - BackpressureController)"

tech-stack:
  added: []
  patterns:
    - "callback-based integration"
    - "constructor injection pattern"
    - "single source of truth for queries"

key-files:
  created:
    - "packages/client/src/sync/QueryManager.ts"
    - "packages/client/src/__tests__/QueryManager.test.ts"
  modified:
    - "packages/client/src/sync/types.ts"
    - "packages/client/src/sync/index.ts"
    - "packages/client/src/SyncEngine.ts"

tags: [refactor, query, subscription, local-query, hybrid-query]

decisions:
  - id: "05-02-01"
    summary: "QueryManager owns queries and hybridQueries Maps"
    rationale: "Single source of truth - query state isolated from SyncEngine"
  - id: "05-02-02"
    summary: "Callback-based integration with SyncEngine for message sending"
    rationale: "Loose coupling - QueryManager doesn't need WebSocketManager directly"
  - id: "05-02-03"
    summary: "resubscribeAll() method for AUTH_ACK handling"
    rationale: "Clean interface for SyncEngine to trigger query resubscription"

metrics:
  duration: "12 min"
  completed: "2026-01-20"
---

# Phase 5 Plan 2: QueryManager Extraction Summary

QueryManager class extracted from SyncEngine - owns query Maps and handles subscriptions, unsubscriptions, and local query execution

## What Was Done

### Task 1: Add QueryManager types

Extended `packages/client/src/sync/types.ts` with:

- `IQueryManager` interface with 10 methods:
  - `getQueries()` - access to queries Map
  - `getHybridQueries()` - access to hybridQueries Map
  - `subscribeToQuery()` / `unsubscribeFromQuery()`
  - `subscribeToHybridQuery()` / `unsubscribeFromHybridQuery()`
  - `getHybridQuery()` - retrieve hybrid query by ID
  - `runLocalQuery()` / `runLocalHybridQuery()` - local storage queries
  - `resubscribeAll()` - re-subscribe after AUTH

- `QueryManagerConfig` interface for dependency injection:
  - `storageAdapter` - for local query execution
  - `sendMessage` - callback for message sending
  - `isAuthenticated` - callback for auth state check

### Task 2: Implement QueryManager class

Created `packages/client/src/sync/QueryManager.ts` (265 lines):

**State ownership:**
- `queries: Map<string, QueryHandle<any>>` - standard queries
- `hybridQueries: Map<string, HybridQueryHandle<any>>` - FTS hybrid queries

**Standard query methods:**
- `subscribeToQuery()` - adds to Map, sends QUERY_SUB if authenticated
- `unsubscribeFromQuery()` - removes from Map, sends QUERY_UNSUB
- `sendQuerySubscription()` - internal message formatting

**Hybrid query methods:**
- `subscribeToHybridQuery()` - adds to Map, sends HYBRID_QUERY_SUBSCRIBE
- `unsubscribeFromHybridQuery()` - removes from Map, sends unsub message
- `sendHybridQuerySubscription()` - internal message formatting

**Local query execution:**
- `runLocalQuery()` - executes against storage adapter with where/predicate filtering
- `runLocalHybridQuery()` - executes with sorting, limiting, score metadata

**Resubscription:**
- `resubscribeAll()` - called by SyncEngine after AUTH_ACK

### Task 3: Integrate QueryManager into SyncEngine

Refactored SyncEngine to delegate all query operations:

**Before:** SyncEngine had ~2336 lines with query logic mixed in
**After:** SyncEngine has ~2050 lines (169 lines removed as query logic moved)

Changes:
- Added QueryManager initialization in constructor
- Removed: `queries` and `hybridQueries` Map fields
- Removed: `subscribeToQuery()`, `unsubscribeFromQuery()`, `sendQuerySubscription()` implementations
- Removed: `subscribeToHybridQuery()`, `unsubscribeFromHybridQuery()`, `sendHybridQuerySubscription()` implementations
- Removed: `runLocalQuery()`, `runLocalHybridQuery()` implementations
- Added: Delegation methods that call `this.queryManager.*`
- Updated: AUTH_ACK handler to use `queryManager.resubscribeAll()`
- Updated: QUERY_RESP/QUERY_UPDATE handlers to use `queryManager.getQueries().get()`
- Updated: Hybrid query response handlers to use `queryManager.getHybridQuery()`

Also fixed blocking issue: Added missing `BackpressureError` import in SyncEngine.

### Task 4: Add QueryManager unit tests

Created `packages/client/src/__tests__/QueryManager.test.ts` with 16 tests:

- Standard query subscription/unsubscription (5 tests)
- Hybrid query subscription/unsubscription (3 tests)
- Local query execution with where clause (2 tests)
- Local query execution with predicate (1 test)
- Local hybrid query with sorting (1 test)
- Local hybrid query with limit (1 test)
- resubscribeAll for standard queries (1 test)
- resubscribeAll for hybrid FTS queries (1 test)

### Task 5: Verification

All 465 client tests pass (1 skipped). No regressions.

## Commits

| Commit | Type | Description |
|--------|------|-------------|
| ba99d0a | feat | Add IQueryManager interface and QueryManagerConfig types |
| a229ee9 | feat | Implement QueryManager class |
| 502cbee | refactor | Integrate QueryManager into SyncEngine |
| 9b75570 | test | Add QueryManager unit tests (16 tests) |

## Verification

- [x] TypeScript compilation: `cd packages/client && npx tsc --noEmit` - PASS
- [x] All 465 client tests pass: `pnpm --filter @topgunbuild/client test` - PASS
- [x] QueryManager file exists at `packages/client/src/sync/QueryManager.ts`
- [x] SyncEngine no longer has direct query Map handling
- [x] Query subscription/unsubscription behavior unchanged
- [x] Local query execution works correctly

## Technical Details

### QueryManager Architecture

```
QueryManager
  ├── Owned State (single source of truth):
  │   ├── queries: Map<string, QueryHandle<any>>
  │   └── hybridQueries: Map<string, HybridQueryHandle<any>>
  │
  ├── Standard Queries:
  │   ├── subscribeToQuery() → add to Map + send QUERY_SUB
  │   ├── unsubscribeFromQuery() → remove from Map + send QUERY_UNSUB
  │   └── getQueries() → read-only access to Map
  │
  ├── Hybrid Queries:
  │   ├── subscribeToHybridQuery() → add to Map + send subscription
  │   ├── unsubscribeFromHybridQuery() → remove from Map + send unsub
  │   ├── getHybridQueries() → read-only access to Map
  │   └── getHybridQuery(id) → get specific hybrid query
  │
  ├── Local Execution:
  │   ├── runLocalQuery() → filter storage by where/predicate
  │   └── runLocalHybridQuery() → filter + sort + limit + score metadata
  │
  └── Resubscription:
      └── resubscribeAll() → called after AUTH_ACK
```

### Integration Pattern

SyncEngine uses callback-based integration:

```typescript
this.queryManager = new QueryManager({
  storageAdapter: this.storageAdapter,
  sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
  isAuthenticated: () => this.isAuthenticated(),
});
```

### Message Formats

**QUERY_SUB:**
```typescript
{ type: 'QUERY_SUB', payload: { queryId, mapName, query: filter } }
```

**QUERY_UNSUB:**
```typescript
{ type: 'QUERY_UNSUB', payload: { queryId } }
```

**HYBRID_QUERY_SUBSCRIBE:**
```typescript
{
  type: 'HYBRID_QUERY_SUBSCRIBE',
  payload: { subscriptionId, mapName, predicate, where, sort, limit, cursor }
}
```

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed missing BackpressureError import**
- **Found during:** Task 3 (SyncEngine integration)
- **Issue:** SyncEngine referenced BackpressureError but import was missing
- **Fix:** Added `import { BackpressureError } from './errors/BackpressureError'`
- **Files modified:** packages/client/src/SyncEngine.ts
- **Commit:** 502cbee

## Next Phase Readiness

Ready for continued Phase 05:
- [x] QueryManager provides clean separation of query concerns
- [x] Pattern consistent with WebSocketManager (callback-based)
- [x] SyncEngine significantly reduced in size
- [x] 05-03 (BackpressureController) already completed in parallel
