---
id: SPEC-009b
parent: SPEC-009
type: refactor
status: draft
priority: high
complexity: medium
created: 2026-01-29
depends_on: [SPEC-009a]
---

# Extract Advanced Feature Handlers from SyncEngine

## Context

This is Part 2 of the SyncEngine refactoring (SPEC-009). This phase extracts three more complex handlers that have request/response patterns with pending request maps and timeout handling.

### Prior Work

Reference: SPEC-009 (parent specification), SPEC-009a (core handlers)

After SPEC-009a, SyncEngine will have:
- TopicManager, LockManager, WriteConcernManager extracted
- Established handler pattern validated

### Why These Handlers Second

- CounterManager, EntryProcessorClient, and SearchClient have similar patterns:
  - Pending request maps
  - Timeout handling
  - Request/response message pairs
- They are independent of each other and the sync handlers (SPEC-009c)
- More complex than core handlers but still well-defined

## Goal Statement

Extract three advanced feature handlers from SyncEngine.ts, reducing the main file by approximately 380 lines while maintaining all existing functionality.

## Task

### 1. CounterManager (~90 lines)

Create `packages/client/src/sync/CounterManager.ts`:

**State to extract from SyncEngine:**
- `counterUpdateListeners: Map<string, Set<(state: any) => void>>` (line 1504)

**Methods to extract:**
- `onCounterUpdate(name, listener)` (lines 1512-1524) - returns unsubscribe function
- `requestCounter(name)` (lines 1530-1537)
- `syncCounter(name, state)` (lines 1544-1560)
- `handleCounterUpdate(name, stateObj)` (lines 1566-1583) - becomes public for message routing

**Message types handled:**
- `COUNTER_UPDATE` (lines 1062-1067)
- `COUNTER_RESPONSE` (lines 1069-1075)

**Config interface:**
```typescript
export interface CounterManagerConfig {
  sendMessage: (message: any, key?: string) => boolean;
  isAuthenticated: () => boolean;
}
```

### 2. EntryProcessorClient (~200 lines)

Create `packages/client/src/sync/EntryProcessorClient.ts`:

**State to extract from SyncEngine:**
- `pendingProcessorRequests: Map<string, {...}>` (lines 1590-1594)
- `pendingBatchProcessorRequests: Map<string, {...}>` (lines 1597-1601)
- `PROCESSOR_TIMEOUT` constant (line 1604)

**Methods to extract:**
- `executeOnKey<V, R>(mapName, key, processor)` (lines 1614-1664)
- `executeOnKeys<V, R>(mapName, keys, processor)` (lines 1674-1729)
- `handleEntryProcessResponse(message)` (lines 1735-1752) - becomes public
- `handleEntryProcessBatchResponse(message)` (lines 1758-1778) - becomes public

**Message types handled:**
- `ENTRY_PROCESS_RESPONSE` (lines 1079-1083)
- `ENTRY_PROCESS_BATCH_RESPONSE` (lines 1085-1089)

**Cleanup method:**
- `close(error?: Error)` - Cancel all pending requests with error, clear maps

**Config interface:**
```typescript
export interface EntryProcessorClientConfig {
  sendMessage: (message: any, key?: string) => boolean;
  isAuthenticated: () => boolean;
  timeoutMs?: number; // Default: 30000
}
```

**Types to import:**
- `EntryProcessorDef`, `EntryProcessorResult`, `EntryProcessKeyResult` from `@topgunbuild/core`

### 3. SearchClient (~90 lines)

Create `packages/client/src/sync/SearchClient.ts`:

**State to extract from SyncEngine:**
- `pendingSearchRequests: Map<string, {...}>` (lines 1842-1846)
- `SEARCH_TIMEOUT` constant (line 1849)

**Methods to extract:**
- `search<T>(mapName, query, options)` (lines 1859-1907)
- `handleSearchResponse(payload)` (lines 1912-1928) - becomes public

**Message type handled:**
- `SEARCH_RESP` (lines 1119-1123)

**Cleanup method:**
- `close(error?: Error)` - Cancel all pending requests with error, clear maps

**Config interface:**
```typescript
export interface SearchClientConfig {
  sendMessage: (message: any, key?: string) => boolean;
  isAuthenticated: () => boolean;
  timeoutMs?: number; // Default: 30000
}
```

**Types to export:**
- `SearchResult<T>` interface (lines 27-32) - move from SyncEngine to SearchClient or types.ts

### Implementation Steps

1. **Add interfaces to `sync/types.ts`:**
   - `ICounterManager`, `CounterManagerConfig`
   - `IEntryProcessorClient`, `EntryProcessorClientConfig`
   - `ISearchClient`, `SearchClientConfig`
   - Move `SearchResult<T>` interface here

2. **Create handler files:**
   - `sync/CounterManager.ts`
   - `sync/EntryProcessorClient.ts`
   - `sync/SearchClient.ts`

3. **Update `sync/index.ts`:**
   - Export new handlers and their types
   - Export `SearchResult` type

4. **Update SyncEngine.ts:**
   - Import new handlers
   - Initialize handlers in constructor
   - Replace method bodies with delegations
   - Update `handleServerMessage()` to delegate to handlers:
     - COUNTER_UPDATE, COUNTER_RESPONSE -> CounterManager
     - ENTRY_PROCESS_RESPONSE, ENTRY_PROCESS_BATCH_RESPONSE -> EntryProcessorClient
     - SEARCH_RESP -> SearchClient
   - Update `close()` to call `entryProcessorClient.close()` and `searchClient.close()`

## Acceptance Criteria

1. [ ] New file `packages/client/src/sync/CounterManager.ts` exists
2. [ ] New file `packages/client/src/sync/EntryProcessorClient.ts` exists
3. [ ] New file `packages/client/src/sync/SearchClient.ts` exists
4. [ ] All handlers implement their respective interfaces
5. [ ] Config interfaces added to `sync/types.ts`
6. [ ] `SearchResult<T>` type exported from `sync/types.ts` or `sync/index.ts`
7. [ ] All handlers exported from `sync/index.ts`
8. [ ] SyncEngine delegates to CounterManager for counter operations
9. [ ] SyncEngine delegates to EntryProcessorClient for entry processor operations
10. [ ] SyncEngine delegates to SearchClient for search operations
11. [ ] Message routing updated for all 5 message types
12. [ ] EntryProcessorClient has `close()` method for cleanup
13. [ ] SearchClient has `close()` method for cleanup
14. [ ] SyncEngine.close() calls cleanup methods on both handlers
15. [ ] All existing tests pass: `pnpm --filter @topgunbuild/client test`
13. [ ] TypeScript compiles without errors: `pnpm --filter @topgunbuild/client build`
14. [ ] No changes to public SyncEngine API

## Constraints

1. **DO NOT** change the WebSocket message protocol
2. **DO NOT** modify test files (tests must pass as-is)
3. **DO NOT** change public method signatures on SyncEngine
4. **DO NOT** introduce circular dependencies between handlers
5. **DO** follow the existing handler pattern from WebSocketManager/QueryManager/BackpressureController
6. **DO** use crypto.randomUUID() for request IDs (same as current implementation)
7. **DO** preserve exact timeout behavior (30000ms default)
8. **DO** import logger directly (not via config)

## Assumptions

1. SPEC-009a has been completed and handlers are available
2. The handler pattern established in SPEC-009a works correctly
3. crypto.randomUUID() is available in the runtime environment
4. EntryProcessorDef, EntryProcessorResult, EntryProcessKeyResult types are exported from @topgunbuild/core
5. SearchOptions type is exported from @topgunbuild/core

## Estimation

**Complexity: medium**

- 3 new handler files (~380 lines total)
- Request/response patterns with timeout handling
- Well-defined dependencies
- Estimated token budget: 50-80k tokens

## Files Summary

| File | Action | Lines |
|------|--------|-------|
| `sync/types.ts` | Modify | +80 (interfaces) |
| `sync/CounterManager.ts` | Create | ~90 |
| `sync/EntryProcessorClient.ts` | Create | ~200 |
| `sync/SearchClient.ts` | Create | ~90 |
| `sync/index.ts` | Modify | +12 (exports) |
| `SyncEngine.ts` | Modify | -320 (delegations) |
