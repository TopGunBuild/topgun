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

      // Verify count matches (using concrete MessageRouter, not IMessageRouter interface)
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
    it('should contain 33 message types', () => {
      expect(CLIENT_MESSAGE_TYPES.length).toBe(33);
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
