---
id: SPEC-009d
parent: SPEC-009
type: refactor
status: draft
priority: high
complexity: medium
created: 2026-01-29
depends_on: [SPEC-009a, SPEC-009b, SPEC-009c]
---

# Create MessageRouter for SyncEngine

## Context

This is Part 4 (final) of the SyncEngine refactoring (SPEC-009). This phase creates a MessageRouter that replaces the large switch statement in `handleServerMessage()` with a type-based routing system.

### Prior Work

Reference: SPEC-009 (parent specification)

After SPEC-009a, SPEC-009b, and SPEC-009c:
- TopicManager handles TOPIC_MESSAGE
- LockManager handles LOCK_GRANTED, LOCK_RELEASED
- WriteConcernManager handles write concern resolution in OP_ACK
- CounterManager handles COUNTER_UPDATE, COUNTER_RESPONSE
- EntryProcessorClient handles ENTRY_PROCESS_RESPONSE, ENTRY_PROCESS_BATCH_RESPONSE
- SearchClient handles SEARCH_RESP
- MerkleSyncHandler handles SYNC_RESP_ROOT, SYNC_RESP_BUCKETS, SYNC_RESP_LEAF, SYNC_RESET_REQUIRED
- ORMapSyncHandler handles ORMAP_SYNC_RESP_ROOT, ORMAP_SYNC_RESP_BUCKETS, ORMAP_SYNC_RESP_LEAF, ORMAP_DIFF_RESPONSE

### Why MessageRouter Last

- All handlers must be in place before routing can be implemented
- The router depends on having handler references
- Final integration step that ties everything together
- Completes the SyncEngine refactoring goal

## Goal Statement

Replace the 480-line `handleServerMessage()` switch statement with a declarative MessageRouter, reducing SyncEngine to under 800 lines and improving maintainability.

## Task

### MessageRouter (~150 lines)

Create `packages/client/src/sync/MessageRouter.ts`:

**Responsibilities:**
- Register handlers for message types
- Route incoming messages to appropriate handlers
- Support handler groups for organization
- Handle message types that remain in SyncEngine via callbacks

**Message Types to Route (35 total):**

| Group | Message Type | Handler |
|-------|-------------|---------|
| BATCH | BATCH | SyncEngine (internal unbatch) |
| AUTH | AUTH_REQUIRED | SyncEngine (callback) |
| AUTH | AUTH_ACK | SyncEngine (callback) |
| AUTH | AUTH_FAIL | SyncEngine (callback) |
| HEARTBEAT | PONG | SyncEngine (no-op, handled by WebSocketManager) |
| SYNC | OP_ACK | SyncEngine (callback + WriteConcernManager) |
| SYNC | SYNC_RESP_ROOT | MerkleSyncHandler |
| SYNC | SYNC_RESP_BUCKETS | MerkleSyncHandler |
| SYNC | SYNC_RESP_LEAF | MerkleSyncHandler |
| SYNC | SYNC_RESET_REQUIRED | MerkleSyncHandler |
| SYNC | ORMAP_SYNC_RESP_ROOT | ORMapSyncHandler |
| SYNC | ORMAP_SYNC_RESP_BUCKETS | ORMapSyncHandler |
| SYNC | ORMAP_SYNC_RESP_LEAF | ORMapSyncHandler |
| SYNC | ORMAP_DIFF_RESPONSE | ORMapSyncHandler |
| QUERY | QUERY_RESP | SyncEngine (callback to QueryManager) |
| QUERY | QUERY_UPDATE | SyncEngine (callback to QueryManager) |
| EVENT | SERVER_EVENT | SyncEngine (callback) |
| EVENT | SERVER_BATCH_EVENT | SyncEngine (callback) |
| TOPIC | TOPIC_MESSAGE | TopicManager |
| LOCK | LOCK_GRANTED | LockManager |
| LOCK | LOCK_RELEASED | LockManager |
| GC | GC_PRUNE | SyncEngine (callback) |
| COUNTER | COUNTER_UPDATE | CounterManager |
| COUNTER | COUNTER_RESPONSE | CounterManager |
| PROCESSOR | ENTRY_PROCESS_RESPONSE | EntryProcessorClient |
| PROCESSOR | ENTRY_PROCESS_BATCH_RESPONSE | EntryProcessorClient |
| RESOLVER | REGISTER_RESOLVER_RESPONSE | ConflictResolverClient (existing) |
| RESOLVER | UNREGISTER_RESOLVER_RESPONSE | ConflictResolverClient (existing) |
| RESOLVER | LIST_RESOLVERS_RESPONSE | ConflictResolverClient (existing) |
| RESOLVER | MERGE_REJECTED | ConflictResolverClient (existing) |
| SEARCH | SEARCH_RESP | SearchClient |
| SEARCH | SEARCH_UPDATE | SyncEngine (no-op, handled by SearchHandle via emitMessage) |
| HYBRID | HYBRID_QUERY_RESP | SyncEngine (handleHybridQueryResponse) |
| HYBRID | HYBRID_QUERY_DELTA | SyncEngine (handleHybridQueryDelta) |

**Note:** HYBRID_QUERY_RESP and HYBRID_QUERY_DELTA currently delegate to `QueryManager`. A future optimization could route directly to `queryManager.handleHybridQueryResponse(...)` from MessageRouter, but the current approach via SyncEngine callbacks is valid and safe.

**Config interface:**
```typescript
export type MessageHandler = (message: any) => Promise<void> | void;

export interface MessageRouterConfig {
  // Handlers registered during construction
  handlers?: Map<string, MessageHandler>;

  // Fallback for unregistered message types
  onUnhandled?: (message: any) => void;
}

export interface IMessageRouter {
  /**
   * Register a handler for a message type.
   */
  registerHandler(type: string, handler: MessageHandler): void;

  /**
   * Register multiple handlers at once.
   */
  registerHandlers(handlers: Record<string, MessageHandler>): void;

  /**
   * Route a message to its registered handler.
   * Returns true if handled, false if no handler found.
   */
  route(message: any): Promise<boolean>;

  /**
   * Check if a handler is registered for a message type.
   */
  hasHandler(type: string): boolean;
}
```

### Implementation Steps

1. **Add interfaces to `sync/types.ts`:**
   - `IMessageRouter`, `MessageRouterConfig`, `MessageHandler`

2. **Create `sync/MessageRouter.ts`:**
   - Implement IMessageRouter
   - Use Map<string, MessageHandler> for routing
   - Handle async handlers properly (await)

3. **Update `sync/index.ts`:**
   - Export MessageRouter and types

4. **Update SyncEngine.ts:**
   - Create MessageRouter in constructor
   - Register all handlers during initialization:
     ```typescript
     this.messageRouter.registerHandlers({
       'TOPIC_MESSAGE': (msg) => this.topicManager.handleTopicMessage(msg.payload),
       'LOCK_GRANTED': (msg) => this.lockManager.handleLockGranted(msg.payload),
       'LOCK_RELEASED': (msg) => this.lockManager.handleLockReleased(msg.payload),
       // ... etc
     });
     ```
   - Replace `handleServerMessage()` switch with:
     ```typescript
     private async handleServerMessage(message: any): Promise<void> {
       this.emitMessage(message);

       // Handle BATCH specially (recursive)
       if (message.type === 'BATCH') {
         // Unbatch and recursively process
         // ... existing unbatch logic
         return;
       }

       // Route to handler
       const handled = await this.messageRouter.route(message);
       if (!handled) {
         logger.warn({ type: message.type }, 'Unhandled message type');
       }

       // Update HLC if message has timestamp
       if (message.timestamp) {
         this.hlc.update(message.timestamp);
         this.lastSyncTimestamp = message.timestamp.millis;
         await this.saveOpLog();
       }
     }
     ```

5. **Refactor remaining inline handlers:**
   - AUTH_REQUIRED, AUTH_ACK, AUTH_FAIL -> callbacks
   - OP_ACK -> callback (keep backpressure and write concern logic)
   - QUERY_RESP, QUERY_UPDATE -> callbacks to QueryManager
   - SERVER_EVENT, SERVER_BATCH_EVENT -> callbacks to applyServerEvent
   - GC_PRUNE -> callback

## Acceptance Criteria

1. [ ] New file `packages/client/src/sync/MessageRouter.ts` exists
2. [ ] MessageRouter implements IMessageRouter interface
3. [ ] Config interface added to `sync/types.ts`
4. [ ] MessageRouter exported from `sync/index.ts`
5. [ ] SyncEngine creates and uses MessageRouter
6. [ ] All 35 message types are routed correctly
7. [ ] `handleServerMessage()` reduced to ~50 lines
8. [ ] SyncEngine.ts total under 800 lines
9. [ ] All existing tests pass: `pnpm --filter @topgunbuild/client test`
10. [ ] TypeScript compiles without errors: `pnpm --filter @topgunbuild/client build`
11. [ ] No changes to public SyncEngine API

## Constraints

1. **DO NOT** change the WebSocket message protocol
2. **DO NOT** modify test files (tests must pass as-is)
3. **DO NOT** change public method signatures on SyncEngine
4. **DO NOT** change message handling semantics
5. **DO** preserve the `emitMessage()` call for EventJournalReader
6. **DO** preserve HLC timestamp updates after all handlers
7. **DO** handle BATCH message type specially (unbatch before routing)
8. **DO** support async handlers (return Promise)

## Assumptions

1. SPEC-009a, SPEC-009b, SPEC-009c have been completed
2. All extracted handlers have public handle* methods
3. The ConflictResolverClient already exists and has handle* methods
4. Message types are strings (message.type)
5. No message type appears in multiple handler groups

## Estimation

**Complexity: medium**

- 1 new file (MessageRouter ~150 lines)
- Significant SyncEngine refactoring (~400 lines removed from switch)
- Wiring all handlers together
- Estimated token budget: 50-80k tokens

## Files Summary

| File | Action | Lines |
|------|--------|-------|
| `sync/types.ts` | Modify | +30 (interfaces) |
| `sync/MessageRouter.ts` | Create | ~150 |
| `sync/index.ts` | Modify | +4 (exports) |
| `SyncEngine.ts` | Modify | -400 (switch -> router) |

## Final Target

After SPEC-009d completion:

| Metric | Before | After | Target |
|--------|--------|-------|--------|
| SyncEngine.ts lines | 2015 | ~750 | <800 |
| Handler files | 3 | 12 | - |
| Message routing | 480-line switch | ~50-line router | - |
