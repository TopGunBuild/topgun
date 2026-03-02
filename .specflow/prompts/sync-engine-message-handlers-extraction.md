# Specification Prompt: Extract SyncEngine Message Handlers

## Context

`packages/client/src/SyncEngine.ts` (1,415 lines) is a well-structured Facade that delegates to 12+ specialized managers. However, the `registerMessageHandlers()` method (lines 297-405) contains 40+ inline handler registrations that:
- Occupy ~110 lines in the main class
- Mix routing logic with the Facade
- Are not testable in isolation
- Make the file harder to navigate

## Objective

Extract message handler registration into a dedicated module to:
1. Reduce SyncEngine by ~110 lines
2. Make message routing testable independently
3. Prepare for Rust-portable enum-based routing
4. Improve code navigation

## Target Architecture

```
SyncEngine.ts (constructor)
  │
  └── this.messageRouter = createClientMessageRouter({
        handlers: this,  // Pass SyncEngine as handler delegate
        managers: { topic, lock, counter, ... },
      });
```

## Requirements

### R1: Create Message Handler Registry
Create `packages/client/src/sync/ClientMessageHandlers.ts`:

```typescript
import { IMessageRouter } from './types';

export interface MessageHandlerDelegates {
  // Auth
  sendAuth: () => void;
  handleAuthAck: () => void;
  handleAuthFail: (msg: any) => void;

  // Sync
  handleOpAck: (msg: any) => void;

  // Query
  handleQueryResp: (msg: any) => void;
  handleQueryUpdate: (msg: any) => void;

  // Events
  handleServerEvent: (msg: any) => Promise<void>;
  handleServerBatchEvent: (msg: any) => Promise<void>;
  handleGcPrune: (msg: any) => Promise<void>;

  // Hybrid Query
  handleHybridQueryResponse: (payload: any) => void;
  handleHybridQueryDelta: (payload: any) => void;
}

export interface ManagerDelegates {
  topicManager: {
    handleTopicMessage: (topic: string, data: any, publisherId: string, timestamp: any) => void;
  };
  lockManager: {
    handleLockGranted: (requestId: string, fencingToken: number) => void;
    handleLockReleased: (requestId: string, success: boolean) => void;
  };
  counterManager: {
    handleCounterUpdate: (name: string, state: any) => void;
  };
  entryProcessorClient: {
    handleEntryProcessResponse: (msg: any) => void;
    handleEntryProcessBatchResponse: (msg: any) => void;
  };
  conflictResolverClient: {
    handleRegisterResponse: (msg: any) => void;
    handleUnregisterResponse: (msg: any) => void;
    handleListResponse: (msg: any) => void;
    handleMergeRejected: (msg: any) => void;
  };
  searchClient: {
    handleSearchResponse: (payload: any) => void;
  };
  merkleSyncHandler: {
    handleSyncRespRoot: (payload: any) => void;
    handleSyncRespBuckets: (payload: any) => void;
    handleSyncRespLeaf: (payload: any) => void;
    handleSyncResetRequired: (payload: any) => void;
  };
  orMapSyncHandler: {
    handleORMapSyncRespRoot: (payload: any) => void;
    handleORMapSyncRespBuckets: (payload: any) => void;
    handleORMapSyncRespLeaf: (payload: any) => void;
    handleORMapDiffResponse: (payload: any) => void;
  };
}

export function registerClientMessageHandlers(
  router: IMessageRouter,
  delegates: MessageHandlerDelegates,
  managers: ManagerDelegates
): void {
  router.registerHandlers({
    // AUTH handlers
    'AUTH_REQUIRED': () => delegates.sendAuth(),
    'AUTH_ACK': () => delegates.handleAuthAck(),
    'AUTH_FAIL': (msg) => delegates.handleAuthFail(msg),

    // HEARTBEAT - no-op (handled by WebSocketManager)
    'PONG': () => {},

    // SYNC handlers
    'OP_ACK': (msg) => delegates.handleOpAck(msg),
    'SYNC_RESP_ROOT': (msg) => managers.merkleSyncHandler.handleSyncRespRoot(msg.payload),
    'SYNC_RESP_BUCKETS': (msg) => managers.merkleSyncHandler.handleSyncRespBuckets(msg.payload),
    'SYNC_RESP_LEAF': (msg) => managers.merkleSyncHandler.handleSyncRespLeaf(msg.payload),
    'SYNC_RESET_REQUIRED': (msg) => managers.merkleSyncHandler.handleSyncResetRequired(msg.payload),

    // ORMAP SYNC handlers
    'ORMAP_SYNC_RESP_ROOT': (msg) => managers.orMapSyncHandler.handleORMapSyncRespRoot(msg.payload),
    'ORMAP_SYNC_RESP_BUCKETS': (msg) => managers.orMapSyncHandler.handleORMapSyncRespBuckets(msg.payload),
    'ORMAP_SYNC_RESP_LEAF': (msg) => managers.orMapSyncHandler.handleORMapSyncRespLeaf(msg.payload),
    'ORMAP_DIFF_RESPONSE': (msg) => managers.orMapSyncHandler.handleORMapDiffResponse(msg.payload),

    // QUERY handlers
    'QUERY_RESP': (msg) => delegates.handleQueryResp(msg),
    'QUERY_UPDATE': (msg) => delegates.handleQueryUpdate(msg),

    // EVENT handlers
    'SERVER_EVENT': (msg) => delegates.handleServerEvent(msg),
    'SERVER_BATCH_EVENT': (msg) => delegates.handleServerBatchEvent(msg),

    // TOPIC handlers
    'TOPIC_MESSAGE': (msg) => {
      const { topic, data, publisherId, timestamp } = msg.payload;
      managers.topicManager.handleTopicMessage(topic, data, publisherId, timestamp);
    },

    // LOCK handlers
    'LOCK_GRANTED': (msg) => {
      const { requestId, fencingToken } = msg.payload;
      managers.lockManager.handleLockGranted(requestId, fencingToken);
    },
    'LOCK_RELEASED': (msg) => {
      const { requestId, success } = msg.payload;
      managers.lockManager.handleLockReleased(requestId, success);
    },

    // GC handler
    'GC_PRUNE': (msg) => delegates.handleGcPrune(msg),

    // COUNTER handlers
    'COUNTER_UPDATE': (msg) => managers.counterManager.handleCounterUpdate(msg.payload.name, msg.payload.state),
    'COUNTER_RESPONSE': (msg) => managers.counterManager.handleCounterUpdate(msg.payload.name, msg.payload.state),

    // PROCESSOR handlers
    'ENTRY_PROCESS_RESPONSE': (msg) => managers.entryProcessorClient.handleEntryProcessResponse(msg),
    'ENTRY_PROCESS_BATCH_RESPONSE': (msg) => managers.entryProcessorClient.handleEntryProcessBatchResponse(msg),

    // RESOLVER handlers
    'REGISTER_RESOLVER_RESPONSE': (msg) => managers.conflictResolverClient.handleRegisterResponse(msg),
    'UNREGISTER_RESOLVER_RESPONSE': (msg) => managers.conflictResolverClient.handleUnregisterResponse(msg),
    'LIST_RESOLVERS_RESPONSE': (msg) => managers.conflictResolverClient.handleListResponse(msg),
    'MERGE_REJECTED': (msg) => managers.conflictResolverClient.handleMergeRejected(msg),

    // SEARCH handlers
    'SEARCH_RESP': (msg) => managers.searchClient.handleSearchResponse(msg.payload),
    'SEARCH_UPDATE': () => {}, // Handled by SearchHandle via emitMessage

    // HYBRID handlers
    'HYBRID_QUERY_RESP': (msg) => delegates.handleHybridQueryResponse(msg.payload),
    'HYBRID_QUERY_DELTA': (msg) => delegates.handleHybridQueryDelta(msg.payload),
  });
}
```

### R2: Update SyncEngine Constructor
Replace inline `registerMessageHandlers()` with call to extracted function:

```typescript
// In constructor (around line 277-281):
import { registerClientMessageHandlers } from './sync/ClientMessageHandlers';

// Initialize MessageRouter (Phase 09d)
this.messageRouter = new MessageRouter({
  onUnhandled: (msg) => logger.warn({ type: msg?.type }, 'Unhandled message type'),
});

// Register handlers via extracted function
registerClientMessageHandlers(this.messageRouter, this, {
  topicManager: this.topicManager,
  lockManager: this.lockManager,
  counterManager: this.counterManager,
  entryProcessorClient: this.entryProcessorClient,
  conflictResolverClient: this.conflictResolverClient,
  searchClient: this.searchClient,
  merkleSyncHandler: this.merkleSyncHandler,
  orMapSyncHandler: this.orMapSyncHandler,
});
```

### R3: Remove Old Method
Delete the `registerMessageHandlers()` method (lines 297-405) from SyncEngine.

### R4: Export from sync/index.ts
Update `packages/client/src/sync/index.ts`:
```typescript
export { registerClientMessageHandlers } from './ClientMessageHandlers';
export type { MessageHandlerDelegates, ManagerDelegates } from './ClientMessageHandlers';
```

### R5: Add Unit Test
Create `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts`:

```typescript
import { MessageRouter } from '../MessageRouter';
import { registerClientMessageHandlers } from '../ClientMessageHandlers';

describe('ClientMessageHandlers', () => {
  it('should register all expected message types', () => {
    const router = new MessageRouter({ onUnhandled: jest.fn() });
    const delegates = createMockDelegates();
    const managers = createMockManagers();

    registerClientMessageHandlers(router, delegates, managers);

    // Verify AUTH_ACK routes correctly
    router.route({ type: 'AUTH_ACK' });
    expect(delegates.handleAuthAck).toHaveBeenCalled();

    // Verify TOPIC_MESSAGE routes correctly
    router.route({
      type: 'TOPIC_MESSAGE',
      payload: { topic: 'test', data: {}, publisherId: 'p1', timestamp: {} },
    });
    expect(managers.topicManager.handleTopicMessage).toHaveBeenCalledWith(
      'test', {}, 'p1', {}
    );
  });

  it('should handle unknown message types via onUnhandled', () => {
    const onUnhandled = jest.fn();
    const router = new MessageRouter({ onUnhandled });
    registerClientMessageHandlers(router, createMockDelegates(), createMockManagers());

    router.route({ type: 'UNKNOWN_TYPE' });
    expect(onUnhandled).toHaveBeenCalled();
  });
});

function createMockDelegates() {
  return {
    sendAuth: jest.fn(),
    handleAuthAck: jest.fn(),
    handleAuthFail: jest.fn(),
    handleOpAck: jest.fn(),
    handleQueryResp: jest.fn(),
    handleQueryUpdate: jest.fn(),
    handleServerEvent: jest.fn(),
    handleServerBatchEvent: jest.fn(),
    handleGcPrune: jest.fn(),
    handleHybridQueryResponse: jest.fn(),
    handleHybridQueryDelta: jest.fn(),
  };
}

function createMockManagers() {
  return {
    topicManager: { handleTopicMessage: jest.fn() },
    lockManager: { handleLockGranted: jest.fn(), handleLockReleased: jest.fn() },
    counterManager: { handleCounterUpdate: jest.fn() },
    entryProcessorClient: {
      handleEntryProcessResponse: jest.fn(),
      handleEntryProcessBatchResponse: jest.fn(),
    },
    conflictResolverClient: {
      handleRegisterResponse: jest.fn(),
      handleUnregisterResponse: jest.fn(),
      handleListResponse: jest.fn(),
      handleMergeRejected: jest.fn(),
    },
    searchClient: { handleSearchResponse: jest.fn() },
    merkleSyncHandler: {
      handleSyncRespRoot: jest.fn(),
      handleSyncRespBuckets: jest.fn(),
      handleSyncRespLeaf: jest.fn(),
      handleSyncResetRequired: jest.fn(),
    },
    orMapSyncHandler: {
      handleORMapSyncRespRoot: jest.fn(),
      handleORMapSyncRespBuckets: jest.fn(),
      handleORMapSyncRespLeaf: jest.fn(),
      handleORMapDiffResponse: jest.fn(),
    },
  };
}
```

## Constraints

- **No Breaking Changes**: SyncEngine public API unchanged
- **No Behavior Changes**: Message routing must work identically
- **Logger calls**: Remove logger.debug calls from handlers (they're in manager methods already)

## Success Criteria

1. SyncEngine.ts reduced by ~110 lines (from 1,415 to ~1,305)
2. `registerMessageHandlers()` method removed from SyncEngine
3. New `ClientMessageHandlers.ts` file with typed interfaces
4. Unit test for message routing
5. All existing tests pass

## Files to Modify

- `packages/client/src/SyncEngine.ts` - Remove method, use extracted function
- `packages/client/src/sync/index.ts` - Add export

## Files to Create

- `packages/client/src/sync/ClientMessageHandlers.ts`
- `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts`

## Rust Portability Notes

This extraction prepares for Rust port by:
1. **Typed interfaces**: `MessageHandlerDelegates` maps to Rust trait
2. **Declarative routing**: Handler map becomes Rust `HashMap<MessageType, Box<dyn Handler>>`
3. **Separated concerns**: Router and handlers can become separate Rust modules
4. **Testable in isolation**: Each handler group testable without full SyncEngine
