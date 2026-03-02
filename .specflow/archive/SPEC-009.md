> **SPLIT:** This specification was decomposed into:
> - SPEC-009a: Extract Core Feature Handlers (TopicManager, LockManager, WriteConcernManager)
> - SPEC-009b: Extract Advanced Feature Handlers (CounterManager, EntryProcessorClient, SearchClient)
> - SPEC-009c: Extract Sync Protocol Handlers (ORMapSyncHandler, MerkleSyncHandler)
> - SPEC-009d: Create MessageRouter for SyncEngine
>
> See child specifications for implementation.

---
id: SPEC-009
type: refactor
status: split
priority: high
complexity: large
created: 2026-01-29
---

# SyncEngine Refactoring - Extract Handlers to Reduce Monolithic Class

## Context

`packages/client/src/SyncEngine.ts` is currently 2015 lines and growing. Similar to the ServerCoordinator refactoring (SPEC-003 through SPEC-008), the client-side SyncEngine has become monolithic with too many responsibilities mixed into a single class.

### Current State

SyncEngine handles:
- Op log management
- Map registration
- Topic management (pub/sub)
- Lock requests
- PN Counter synchronization
- Entry Processor execution
- Full-text search
- Write Concern tracking
- ORMap sync
- Hybrid query responses
- Conflict resolution client
- Failover support
- Message event routing (large switch statement with 35+ message types)

### Prior Work

Partial extraction has already been done in `packages/client/src/sync/`:
- `WebSocketManager.ts` - WebSocket connection lifecycle, heartbeat, reconnection
- `QueryManager.ts` - Query subscriptions, local query execution, hybrid queries
- `BackpressureController.ts` - Flow control, pause/resume, drop-oldest strategy

### Goal

Continue the refactoring pattern established by the ServerCoordinator work. Extract focused handlers from SyncEngine to:
1. Reduce SyncEngine to under 800 lines (core orchestration only)
2. Improve testability through dependency injection
3. Follow the established Config-based handler pattern

## Goal-Backward Analysis

### Goal Statement
SyncEngine should be a thin orchestration layer that delegates to specialized handlers, similar to how ServerCoordinator delegates to BroadcastHandler, GCHandler, ClusterEventHandler, etc.

### Observable Truths (when done)
1. SyncEngine.ts is under 800 lines
2. Each handler has a single, clear responsibility
3. All existing tests pass without modification
4. TypeScript compiles without errors
5. New handlers follow the Config-based dependency injection pattern
6. Message routing uses delegation rather than a giant switch statement

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `sync/TopicManager.ts` | Topic subscription, publication, offline queue |
| `sync/LockManager.ts` | Distributed lock request/release |
| `sync/CounterManager.ts` | PN Counter sync and update listeners |
| `sync/EntryProcessorClient.ts` | Entry processor execution and response handling |
| `sync/SearchClient.ts` | Full-text search requests and responses |
| `sync/WriteConcernManager.ts` | Write concern promise tracking |
| `sync/ORMapSyncHandler.ts` | ORMap-specific sync messages |
| `sync/MessageRouter.ts` | Message type routing (replaces switch statement) |

### Required Wiring

1. SyncEngine creates and owns all managers/handlers
2. Managers receive callbacks for sendMessage, isAuthenticated, etc.
3. MessageRouter delegates message types to appropriate handlers
4. Handlers can emit events that SyncEngine listens to

### Key Links (fragile/critical)

1. **Message routing completeness** - All 35+ message types must be handled
2. **Authentication state** - Managers need to know when to send subscriptions
3. **Op log consistency** - BackpressureController and WriteConcernManager share opLog reference

## Task

Extract handlers from SyncEngine following the established pattern:

### Phase 1: Core Feature Handlers (SPEC-009a)
1. **TopicManager** - ~100 lines
   - `topics` Map
   - `topicQueue` and `topicQueueConfig`
   - `subscribeToTopic()`, `unsubscribeFromTopic()`, `publishTopic()`
   - `queueTopicMessage()`, `flushTopicQueue()`, `sendTopicSubscription()`
   - `getTopicQueueStatus()`
   - Handle TOPIC_MESSAGE

2. **LockManager** - ~70 lines
   - `pendingLockRequests` Map
   - `requestLock()`, `releaseLock()`
   - Handle LOCK_GRANTED, LOCK_RELEASED

3. **WriteConcernManager** - ~60 lines
   - `pendingWriteConcernPromises` Map
   - `registerWriteConcernPromise()`, `resolveWriteConcernPromise()`
   - `cancelAllWriteConcernPromises()`

### Phase 2: Advanced Feature Handlers (SPEC-009b)
4. **CounterManager** - ~90 lines
   - `counterUpdateListeners` Map
   - `onCounterUpdate()`, `requestCounter()`, `syncCounter()`
   - `handleCounterUpdate()`
   - Handle COUNTER_UPDATE, COUNTER_RESPONSE

5. **EntryProcessorClient** - ~200 lines
   - `pendingProcessorRequests` Map
   - `pendingBatchProcessorRequests` Map
   - `executeOnKey()`, `executeOnKeys()`
   - `handleEntryProcessResponse()`, `handleEntryProcessBatchResponse()`
   - Handle ENTRY_PROCESS_RESPONSE, ENTRY_PROCESS_BATCH_RESPONSE

6. **SearchClient** - ~90 lines
   - `pendingSearchRequests` Map
   - `search()`
   - `handleSearchResponse()`
   - Handle SEARCH_RESP

### Phase 3: Sync Handlers (SPEC-009c)
7. **ORMapSyncHandler** - ~130 lines
   - Handle ORMAP_SYNC_RESP_ROOT, ORMAP_SYNC_RESP_BUCKETS, ORMAP_SYNC_RESP_LEAF
   - Handle ORMAP_DIFF_RESPONSE
   - `pushORMapDiff()`

8. **MerkleSyncHandler** - ~70 lines
   - Handle SYNC_RESP_ROOT, SYNC_RESP_BUCKETS, SYNC_RESP_LEAF
   - Handle SYNC_RESET_REQUIRED

### Phase 4: Message Router (SPEC-009d)
9. **MessageRouter** - ~150 lines
   - Replace `handleServerMessage()` switch statement
   - Route message types to appropriate handlers
   - Provide `registerHandler(type, handler)` API
   - Support handler groups (AUTH, QUERY, SYNC, TOPIC, LOCK, etc.)

## Acceptance Criteria

1. [ ] SyncEngine.ts reduced to under 800 lines
2. [ ] All new handlers in `packages/client/src/sync/` directory
3. [ ] All new handlers exported from `sync/index.ts`
4. [ ] All handlers implement interfaces defined in `sync/types.ts`
5. [ ] All handlers follow Config-based dependency injection pattern
6. [ ] All existing tests pass: `pnpm --filter @topgunbuild/client test`
7. [ ] TypeScript compiles without errors: `pnpm --filter @topgunbuild/client build`
8. [ ] No changes to public SyncEngine API (external callers unaffected)

## Constraints

1. **DO NOT** change the WebSocket message protocol
2. **DO NOT** modify test files (tests must pass as-is)
3. **DO NOT** change the public API of SyncEngine
4. **DO NOT** introduce circular dependencies between handlers
5. **DO** follow the existing handler pattern from WebSocketManager/QueryManager/BackpressureController
6. **DO** use callbacks for operations that remain in SyncEngine
7. **DO** preserve exact message handling semantics

## Assumptions

1. The existing WebSocketManager, QueryManager, BackpressureController patterns are correct and should be followed
2. All message types handled in the current switch statement are documented by the message type constants
3. Handlers can receive `sendMessage` callback to communicate with server
4. The `isAuthenticated()` pattern works for gating server communication
5. Conflict resolution client (ConflictResolverClient) is already a separate class and doesn't need extraction

## Estimation

**Complexity: large**

This specification encompasses:
- 8 new handler files (~800 lines total new code)
- Significant SyncEngine refactoring (~1200 lines to remove/reorganize)
- Message routing restructure
- Type definitions for all new handlers

**Estimated total: >150k tokens**

**Recommendation:** Split into 4 child specifications (SPEC-009a through SPEC-009d) before implementation.

---

## Handler Extraction Details

### Existing Extracted (reference)

| Handler | Lines | Responsibility |
|---------|-------|----------------|
| WebSocketManager | 495 | Connection, heartbeat, reconnection |
| QueryManager | 330 | Query subscriptions, local queries |
| BackpressureController | 260 | Flow control, pause/resume |

### Proposed Extractions

| Handler | Est. Lines | Responsibility |
|---------|------------|----------------|
| TopicManager | 100 | Topic pub/sub, offline queue |
| LockManager | 70 | Distributed lock requests |
| WriteConcernManager | 60 | Write concern promise tracking |
| CounterManager | 90 | PN Counter synchronization |
| EntryProcessorClient | 200 | Entry processor execution |
| SearchClient | 90 | Full-text search requests |
| ORMapSyncHandler | 130 | ORMap sync protocol |
| MerkleSyncHandler | 70 | LWWMap Merkle sync |
| MessageRouter | 150 | Message type routing |

**Total extraction: ~960 lines**
**SyncEngine target: <800 lines** (from 2015)

---

## Split History

**Split Date:** 2026-01-29

**Child Specifications Created:**

| ID | Title | Size | Depends On |
|----|-------|------|------------|
| SPEC-009a | Extract Core Feature Handlers | small | - |
| SPEC-009b | Extract Advanced Feature Handlers | medium | SPEC-009a |
| SPEC-009c | Extract Sync Protocol Handlers | small | SPEC-009a |
| SPEC-009d | Create MessageRouter | medium | SPEC-009a, SPEC-009b, SPEC-009c |

**Dependency Graph:**
```
SPEC-009a (Core: Topic, Lock, WriteConcern)
    ↓
SPEC-009b (Advanced: Counter, EntryProcessor, Search)
    ↓
SPEC-009c (Sync: Merkle, ORMap) ← also depends on SPEC-009a
    ↓
SPEC-009d (MessageRouter) ← depends on all above
```

**Rationale:**
- SPEC-009a establishes the handler pattern with simpler handlers
- SPEC-009b and SPEC-009c can be implemented in parallel after SPEC-009a
- SPEC-009d must be last as it wires all handlers together
