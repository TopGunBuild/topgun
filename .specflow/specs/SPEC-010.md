# SPEC-010: Extract SyncEngine Message Handlers

```yaml
id: SPEC-010
type: refactor
status: draft
priority: medium
complexity: small
created: 2026-01-30
```

## Context

`packages/client/src/SyncEngine.ts` (1,416 lines) is a well-structured Facade that delegates to 12+ specialized managers. SPEC-009d introduced `MessageRouter` for declarative message routing, replacing a ~330-line switch statement with a 19-line `handleServerMessage()` method.

However, the `registerMessageHandlers()` method (lines 297-405) contains ~35 inline handler registrations that:
- Occupy ~110 lines in the main class
- Mix routing configuration with the Facade
- Cannot be tested in isolation from SyncEngine
- Make the file harder to navigate

This specification extracts handler registration into a dedicated module, following the established handler extraction pattern from SPEC-003 series.

### Prior Work

- **SPEC-009d:** Created `MessageRouter` with `registerHandlers()` API
- **SPEC-009a/b/c:** Extracted TopicManager, LockManager, CounterManager, etc. - established pattern

## Task

Extract the `registerMessageHandlers()` method into a standalone `ClientMessageHandlers.ts` module that:
1. Defines typed interfaces for SyncEngine delegate methods and manager handles
2. Provides a `registerClientMessageHandlers()` function that configures all message routing
3. Enables isolated testing of message handler registration

## Goal Analysis

### Goal Statement
Message handler registration is extracted from SyncEngine into a testable, maintainable module.

### Observable Truths (when done)
1. `registerMessageHandlers()` method no longer exists in SyncEngine.ts
2. SyncEngine constructor calls `registerClientMessageHandlers()` from external module
3. All 35 message types are registered (AUTH_REQUIRED, AUTH_ACK, AUTH_FAIL, PONG, OP_ACK, SYNC_RESP_ROOT, SYNC_RESP_BUCKETS, SYNC_RESP_LEAF, SYNC_RESET_REQUIRED, ORMAP_SYNC_RESP_ROOT, ORMAP_SYNC_RESP_BUCKETS, ORMAP_SYNC_RESP_LEAF, ORMAP_DIFF_RESPONSE, QUERY_RESP, QUERY_UPDATE, SERVER_EVENT, SERVER_BATCH_EVENT, TOPIC_MESSAGE, LOCK_GRANTED, LOCK_RELEASED, GC_PRUNE, COUNTER_UPDATE, COUNTER_RESPONSE, ENTRY_PROCESS_RESPONSE, ENTRY_PROCESS_BATCH_RESPONSE, REGISTER_RESOLVER_RESPONSE, UNREGISTER_RESOLVER_RESPONSE, LIST_RESOLVERS_RESPONSE, MERGE_REJECTED, SEARCH_RESP, SEARCH_UPDATE, HYBRID_QUERY_RESP, HYBRID_QUERY_DELTA)
4. Unit test verifies all message types are registered
5. Existing SyncEngine tests pass unchanged

### Required Artifacts
- `packages/client/src/sync/ClientMessageHandlers.ts` (CREATE)
- `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts` (CREATE)
- `packages/client/src/SyncEngine.ts` (MODIFY)
- `packages/client/src/sync/index.ts` (MODIFY)

### Key Links
- ClientMessageHandlers imports from `./types` (IMessageRouter, MessageHandler)
- SyncEngine imports `registerClientMessageHandlers` from `./sync`
- Test imports both MessageRouter and registerClientMessageHandlers

## Requirements

### R1: Create Message Handler Registry Module

Create `packages/client/src/sync/ClientMessageHandlers.ts`:

```typescript
/**
 * Client message handler registration.
 * Configures all message type -> handler mappings for SyncEngine.
 */

import type { IMessageRouter } from './types';

/**
 * SyncEngine methods used as handler delegates.
 * These methods are called directly from message handlers.
 */
export interface MessageHandlerDelegates {
  sendAuth(): Promise<void>;
  handleAuthAck(): void;
  handleAuthFail(message: any): void;
  handleOpAck(message: any): void;
  handleQueryResp(message: any): void;
  handleQueryUpdate(message: any): void;
  handleServerEvent(message: any): Promise<void>;
  handleServerBatchEvent(message: any): Promise<void>;
  handleGcPrune(message: any): Promise<void>;
  handleHybridQueryResponse(payload: any): void;
  handleHybridQueryDelta(payload: any): void;
}

/**
 * Manager instances that receive delegated message handling.
 */
export interface ManagerDelegates {
  topicManager: {
    handleTopicMessage(topic: string, data: any, publisherId: string, timestamp: any): void;
  };
  lockManager: {
    handleLockGranted(requestId: string, fencingToken: number): void;
    handleLockReleased(requestId: string, success: boolean): void;
  };
  counterManager: {
    handleCounterUpdate(name: string, state: any): void;
  };
  entryProcessorClient: {
    handleEntryProcessResponse(message: any): void;
    handleEntryProcessBatchResponse(message: any): void;
  };
  conflictResolverClient: {
    handleRegisterResponse(message: any): void;
    handleUnregisterResponse(message: any): void;
    handleListResponse(message: any): void;
    handleMergeRejected(message: any): void;
  };
  searchClient: {
    handleSearchResponse(payload: any): void;
  };
  merkleSyncHandler: {
    handleSyncRespRoot(payload: any): void;
    handleSyncRespBuckets(payload: any): void;
    handleSyncRespLeaf(payload: any): void;
    handleSyncResetRequired(payload: any): void;
  };
  orMapSyncHandler: {
    handleORMapSyncRespRoot(payload: any): void;
    handleORMapSyncRespBuckets(payload: any): void;
    handleORMapSyncRespLeaf(payload: any): void;
    handleORMapDiffResponse(payload: any): void;
  };
}

/**
 * All expected client message types.
 * Used for testing that all types are registered.
 */
export const CLIENT_MESSAGE_TYPES = [
  'AUTH_REQUIRED', 'AUTH_ACK', 'AUTH_FAIL',
  'PONG',
  'OP_ACK',
  'SYNC_RESP_ROOT', 'SYNC_RESP_BUCKETS', 'SYNC_RESP_LEAF', 'SYNC_RESET_REQUIRED',
  'ORMAP_SYNC_RESP_ROOT', 'ORMAP_SYNC_RESP_BUCKETS', 'ORMAP_SYNC_RESP_LEAF', 'ORMAP_DIFF_RESPONSE',
  'QUERY_RESP', 'QUERY_UPDATE',
  'SERVER_EVENT', 'SERVER_BATCH_EVENT',
  'TOPIC_MESSAGE',
  'LOCK_GRANTED', 'LOCK_RELEASED',
  'GC_PRUNE',
  'COUNTER_UPDATE', 'COUNTER_RESPONSE',
  'ENTRY_PROCESS_RESPONSE', 'ENTRY_PROCESS_BATCH_RESPONSE',
  'REGISTER_RESOLVER_RESPONSE', 'UNREGISTER_RESOLVER_RESPONSE', 'LIST_RESOLVERS_RESPONSE', 'MERGE_REJECTED',
  'SEARCH_RESP', 'SEARCH_UPDATE',
  'HYBRID_QUERY_RESP', 'HYBRID_QUERY_DELTA',
] as const;

/**
 * Register all client message handlers with the router.
 *
 * @param router - MessageRouter instance
 * @param delegates - SyncEngine handler methods
 * @param managers - Manager instances for delegation
 */
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

    // HEARTBEAT - handled by WebSocketManager, no-op here
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
    'COUNTER_UPDATE': (msg) => {
      const { name, state } = msg.payload;
      managers.counterManager.handleCounterUpdate(name, state);
    },
    'COUNTER_RESPONSE': (msg) => {
      const { name, state } = msg.payload;
      managers.counterManager.handleCounterUpdate(name, state);
    },

    // PROCESSOR handlers
    'ENTRY_PROCESS_RESPONSE': (msg) => {
      managers.entryProcessorClient.handleEntryProcessResponse(msg);
    },
    'ENTRY_PROCESS_BATCH_RESPONSE': (msg) => {
      managers.entryProcessorClient.handleEntryProcessBatchResponse(msg);
    },

    // RESOLVER handlers
    'REGISTER_RESOLVER_RESPONSE': (msg) => {
      managers.conflictResolverClient.handleRegisterResponse(msg);
    },
    'UNREGISTER_RESOLVER_RESPONSE': (msg) => {
      managers.conflictResolverClient.handleUnregisterResponse(msg);
    },
    'LIST_RESOLVERS_RESPONSE': (msg) => {
      managers.conflictResolverClient.handleListResponse(msg);
    },
    'MERGE_REJECTED': (msg) => {
      managers.conflictResolverClient.handleMergeRejected(msg);
    },

    // SEARCH handlers
    'SEARCH_RESP': (msg) => {
      managers.searchClient.handleSearchResponse(msg.payload);
    },
    'SEARCH_UPDATE': () => {
      // SEARCH_UPDATE is handled by SearchHandle via emitMessage, no-op here
    },

    // HYBRID handlers
    'HYBRID_QUERY_RESP': (msg) => delegates.handleHybridQueryResponse(msg.payload),
    'HYBRID_QUERY_DELTA': (msg) => delegates.handleHybridQueryDelta(msg.payload),
  });
}
```

### R2: Update SyncEngine Constructor

In `packages/client/src/SyncEngine.ts`:

1. Add import:
```typescript
import { registerClientMessageHandlers } from './sync/ClientMessageHandlers';
```

2. Replace `registerMessageHandlers()` call (line ~281) with:
```typescript
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

### R3: Remove registerMessageHandlers Method

Delete the `registerMessageHandlers()` method (lines ~297-405) from SyncEngine.ts.

### R4: Export from sync/index.ts

Add to `packages/client/src/sync/index.ts`:
```typescript
export {
  registerClientMessageHandlers,
  CLIENT_MESSAGE_TYPES,
  type MessageHandlerDelegates,
  type ManagerDelegates,
} from './ClientMessageHandlers';
```

### R5: Add Unit Test

Create `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts`:

```typescript
import { MessageRouter } from '../MessageRouter';
import { registerClientMessageHandlers, CLIENT_MESSAGE_TYPES } from '../ClientMessageHandlers';

describe('ClientMessageHandlers', () => {
  describe('registerClientMessageHandlers', () => {
    it('should register all expected message types', () => {
      const router = new MessageRouter();

      // Create mock delegates
      const mockDelegates = {
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

      // Create mock managers
      const mockManagers = {
        topicManager: { handleTopicMessage: jest.fn() },
        lockManager: { handleLockGranted: jest.fn(), handleLockReleased: jest.fn() },
        counterManager: { handleCounterUpdate: jest.fn() },
        entryProcessorClient: { handleEntryProcessResponse: jest.fn(), handleEntryProcessBatchResponse: jest.fn() },
        conflictResolverClient: { handleRegisterResponse: jest.fn(), handleUnregisterResponse: jest.fn(), handleListResponse: jest.fn(), handleMergeRejected: jest.fn() },
        searchClient: { handleSearchResponse: jest.fn() },
        merkleSyncHandler: { handleSyncRespRoot: jest.fn(), handleSyncRespBuckets: jest.fn(), handleSyncRespLeaf: jest.fn(), handleSyncResetRequired: jest.fn() },
        orMapSyncHandler: { handleORMapSyncRespRoot: jest.fn(), handleORMapSyncRespBuckets: jest.fn(), handleORMapSyncRespLeaf: jest.fn(), handleORMapDiffResponse: jest.fn() },
      };

      registerClientMessageHandlers(router, mockDelegates, mockManagers);

      // Verify all message types are registered
      for (const type of CLIENT_MESSAGE_TYPES) {
        expect(router.hasHandler(type)).toBe(true);
      }

      // Verify count matches
      expect(router.handlerCount).toBe(CLIENT_MESSAGE_TYPES.length);
    });

    it('should call onUnhandled for unknown message types', async () => {
      const onUnhandled = jest.fn();
      const router = new MessageRouter({ onUnhandled });

      const mockDelegates = {
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

      const mockManagers = {
        topicManager: { handleTopicMessage: jest.fn() },
        lockManager: { handleLockGranted: jest.fn(), handleLockReleased: jest.fn() },
        counterManager: { handleCounterUpdate: jest.fn() },
        entryProcessorClient: { handleEntryProcessResponse: jest.fn(), handleEntryProcessBatchResponse: jest.fn() },
        conflictResolverClient: { handleRegisterResponse: jest.fn(), handleUnregisterResponse: jest.fn(), handleListResponse: jest.fn(), handleMergeRejected: jest.fn() },
        searchClient: { handleSearchResponse: jest.fn() },
        merkleSyncHandler: { handleSyncRespRoot: jest.fn(), handleSyncRespBuckets: jest.fn(), handleSyncRespLeaf: jest.fn(), handleSyncResetRequired: jest.fn() },
        orMapSyncHandler: { handleORMapSyncRespRoot: jest.fn(), handleORMapSyncRespBuckets: jest.fn(), handleORMapSyncRespLeaf: jest.fn(), handleORMapDiffResponse: jest.fn() },
      };

      registerClientMessageHandlers(router, mockDelegates, mockManagers);

      const result = await router.route({ type: 'UNKNOWN_TYPE' });
      expect(result).toBe(false);
      expect(onUnhandled).toHaveBeenCalledWith({ type: 'UNKNOWN_TYPE' });
    });

    it('should route AUTH_ACK to handleAuthAck delegate', async () => {
      const router = new MessageRouter();

      const mockDelegates = {
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

      const mockManagers = {
        topicManager: { handleTopicMessage: jest.fn() },
        lockManager: { handleLockGranted: jest.fn(), handleLockReleased: jest.fn() },
        counterManager: { handleCounterUpdate: jest.fn() },
        entryProcessorClient: { handleEntryProcessResponse: jest.fn(), handleEntryProcessBatchResponse: jest.fn() },
        conflictResolverClient: { handleRegisterResponse: jest.fn(), handleUnregisterResponse: jest.fn(), handleListResponse: jest.fn(), handleMergeRejected: jest.fn() },
        searchClient: { handleSearchResponse: jest.fn() },
        merkleSyncHandler: { handleSyncRespRoot: jest.fn(), handleSyncRespBuckets: jest.fn(), handleSyncRespLeaf: jest.fn(), handleSyncResetRequired: jest.fn() },
        orMapSyncHandler: { handleORMapSyncRespRoot: jest.fn(), handleORMapSyncRespBuckets: jest.fn(), handleORMapSyncRespLeaf: jest.fn(), handleORMapDiffResponse: jest.fn() },
      };

      registerClientMessageHandlers(router, mockDelegates, mockManagers);

      await router.route({ type: 'AUTH_ACK' });
      expect(mockDelegates.handleAuthAck).toHaveBeenCalled();
    });

    it('should route TOPIC_MESSAGE to topicManager', async () => {
      const router = new MessageRouter();

      const mockDelegates = {
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

      const mockManagers = {
        topicManager: { handleTopicMessage: jest.fn() },
        lockManager: { handleLockGranted: jest.fn(), handleLockReleased: jest.fn() },
        counterManager: { handleCounterUpdate: jest.fn() },
        entryProcessorClient: { handleEntryProcessResponse: jest.fn(), handleEntryProcessBatchResponse: jest.fn() },
        conflictResolverClient: { handleRegisterResponse: jest.fn(), handleUnregisterResponse: jest.fn(), handleListResponse: jest.fn(), handleMergeRejected: jest.fn() },
        searchClient: { handleSearchResponse: jest.fn() },
        merkleSyncHandler: { handleSyncRespRoot: jest.fn(), handleSyncRespBuckets: jest.fn(), handleSyncRespLeaf: jest.fn(), handleSyncResetRequired: jest.fn() },
        orMapSyncHandler: { handleORMapSyncRespRoot: jest.fn(), handleORMapSyncRespBuckets: jest.fn(), handleORMapSyncRespLeaf: jest.fn(), handleORMapDiffResponse: jest.fn() },
      };

      registerClientMessageHandlers(router, mockDelegates, mockManagers);

      await router.route({
        type: 'TOPIC_MESSAGE',
        payload: { topic: 'chat', data: 'hello', publisherId: 'node1', timestamp: 123 }
      });

      expect(mockManagers.topicManager.handleTopicMessage).toHaveBeenCalledWith(
        'chat', 'hello', 'node1', 123
      );
    });
  });

  describe('CLIENT_MESSAGE_TYPES', () => {
    it('should contain 35 message types', () => {
      expect(CLIENT_MESSAGE_TYPES.length).toBe(35);
    });

    it('should include all auth types', () => {
      expect(CLIENT_MESSAGE_TYPES).toContain('AUTH_REQUIRED');
      expect(CLIENT_MESSAGE_TYPES).toContain('AUTH_ACK');
      expect(CLIENT_MESSAGE_TYPES).toContain('AUTH_FAIL');
    });

    it('should include all sync types', () => {
      expect(CLIENT_MESSAGE_TYPES).toContain('SYNC_RESP_ROOT');
      expect(CLIENT_MESSAGE_TYPES).toContain('SYNC_RESP_BUCKETS');
      expect(CLIENT_MESSAGE_TYPES).toContain('SYNC_RESP_LEAF');
      expect(CLIENT_MESSAGE_TYPES).toContain('SYNC_RESET_REQUIRED');
    });
  });
});
```

## Files

| File | Action | Description |
|------|--------|-------------|
| `packages/client/src/sync/ClientMessageHandlers.ts` | CREATE | Message handler registration module |
| `packages/client/src/sync/__tests__/ClientMessageHandlers.test.ts` | CREATE | Unit tests for handler registration |
| `packages/client/src/SyncEngine.ts` | MODIFY | Remove method, use extracted function |
| `packages/client/src/sync/index.ts` | MODIFY | Add export for ClientMessageHandlers |

## Acceptance Criteria

1. `registerMessageHandlers()` method removed from SyncEngine.ts
2. `ClientMessageHandlers.ts` created with `MessageHandlerDelegates` interface
3. `ClientMessageHandlers.ts` created with `ManagerDelegates` interface
4. `ClientMessageHandlers.ts` exports `CLIENT_MESSAGE_TYPES` constant (35 types)
5. `registerClientMessageHandlers()` function registers all 35 message types
6. SyncEngine constructor calls `registerClientMessageHandlers()` with correct arguments
7. `sync/index.ts` exports `registerClientMessageHandlers`, `CLIENT_MESSAGE_TYPES`, and interfaces
8. Unit test verifies all 35 message types are registered
9. Unit test verifies unknown types trigger `onUnhandled`
10. Unit test verifies delegate routing (AUTH_ACK -> handleAuthAck)
11. Unit test verifies manager routing (TOPIC_MESSAGE -> topicManager)
12. SyncEngine.ts reduced by ~110 lines (from ~1,416 to ~1,306)
13. Client package builds successfully
14. All existing SyncEngine tests pass unchanged

## Constraints

- **No Breaking Changes:** SyncEngine public API remains unchanged
- **No Behavior Changes:** Message routing must work identically to current implementation
- **No Logger Calls:** Remove `logger.debug` calls from handlers (they exist in manager methods already)
- **Follow Existing Patterns:** Match style of existing sync module files

## Assumptions

1. The 35 message types in `registerMessageHandlers()` represent the complete set (verified by line count)
2. TypeScript interfaces with `any` types are acceptable for initial extraction (can be refined later)
3. Test file location follows existing pattern (`sync/__tests__/`)
4. ConflictResolverClient is a direct dependency of SyncEngine (not from sync module)
