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
