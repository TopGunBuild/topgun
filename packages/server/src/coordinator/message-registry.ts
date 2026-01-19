/**
 * MessageRegistry - Maps message types to handler functions
 *
 * This module replaces the giant switch statement in ServerCoordinator.handleMessage
 * with a registry pattern that provides:
 * - Clear mapping of message types to handlers
 * - Type-safe handler definitions
 * - Easy extensibility for new message types
 *
 * Usage:
 * 1. Create registry with handler functions
 * 2. Look up handler by message.type
 * 3. Call handler(client, message)
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import type { ClientConnection } from './types';

/**
 * Handler function for a specific message type.
 * Can be async or sync - both are supported.
 */
export type MessageHandler = (client: ClientConnection, message: any) => Promise<void> | void;

/**
 * Registry mapping message type strings to handler functions.
 */
export interface MessageRegistry {
    [messageType: string]: MessageHandler;
}

/**
 * Handler definitions for createMessageRegistry.
 * Each handler corresponds to a message type in the protocol.
 */
export interface MessageHandlers {
    // Query operations
    onQuerySub: MessageHandler;
    onQueryUnsub: MessageHandler;

    // CRDT operations
    onClientOp: MessageHandler;
    onOpBatch: MessageHandler;

    // LWW Sync protocol
    onSyncInit: MessageHandler;
    onMerkleReqBucket: MessageHandler;

    // ORMap Sync protocol
    onORMapSyncInit: MessageHandler;
    onORMapMerkleReqBucket: MessageHandler;
    onORMapDiffRequest: MessageHandler;
    onORMapPushDiff: MessageHandler;

    // Lock operations
    onLockRequest: MessageHandler;
    onLockRelease: MessageHandler;

    // Topic operations (pub/sub)
    onTopicSub: MessageHandler;
    onTopicUnsub: MessageHandler;
    onTopicPub: MessageHandler;

    // PN Counter operations
    onCounterRequest: MessageHandler;
    onCounterSync: MessageHandler;

    // Entry processor operations
    onEntryProcess: MessageHandler;
    onEntryProcessBatch: MessageHandler;

    // Conflict resolver operations
    onRegisterResolver: MessageHandler;
    onUnregisterResolver: MessageHandler;
    onListResolvers: MessageHandler;

    // Partition operations
    onPartitionMapRequest: MessageHandler;

    // Full-text search operations
    onSearch: MessageHandler;
    onSearchSub: MessageHandler;
    onSearchUnsub: MessageHandler;

    // Event journal operations
    onJournalSubscribe: MessageHandler;
    onJournalUnsubscribe: MessageHandler;
    onJournalRead: MessageHandler;
}

/**
 * Create a message registry mapping message types to handler functions.
 * This replaces the 30+ case switch statement in handleMessage.
 *
 * @param handlers Object containing handler functions for each message type
 * @returns Registry object for O(1) message type lookup
 *
 * Note: AUTH is handled separately before the registry (pre-authentication).
 * PING is also handled separately for performance.
 */
export function createMessageRegistry(handlers: MessageHandlers): MessageRegistry {
    return {
        // Query operations
        'QUERY_SUB': handlers.onQuerySub,
        'QUERY_UNSUB': handlers.onQueryUnsub,

        // CRDT operations
        'CLIENT_OP': handlers.onClientOp,
        'OP_BATCH': handlers.onOpBatch,

        // LWW Sync protocol
        'SYNC_INIT': handlers.onSyncInit,
        'MERKLE_REQ_BUCKET': handlers.onMerkleReqBucket,

        // ORMap Sync protocol
        'ORMAP_SYNC_INIT': handlers.onORMapSyncInit,
        'ORMAP_MERKLE_REQ_BUCKET': handlers.onORMapMerkleReqBucket,
        'ORMAP_DIFF_REQUEST': handlers.onORMapDiffRequest,
        'ORMAP_PUSH_DIFF': handlers.onORMapPushDiff,

        // Lock operations
        'LOCK_REQUEST': handlers.onLockRequest,
        'LOCK_RELEASE': handlers.onLockRelease,

        // Topic operations
        'TOPIC_SUB': handlers.onTopicSub,
        'TOPIC_UNSUB': handlers.onTopicUnsub,
        'TOPIC_PUB': handlers.onTopicPub,

        // PN Counter operations
        'COUNTER_REQUEST': handlers.onCounterRequest,
        'COUNTER_SYNC': handlers.onCounterSync,

        // Entry processor operations
        'ENTRY_PROCESS': handlers.onEntryProcess,
        'ENTRY_PROCESS_BATCH': handlers.onEntryProcessBatch,

        // Conflict resolver operations
        'REGISTER_RESOLVER': handlers.onRegisterResolver,
        'UNREGISTER_RESOLVER': handlers.onUnregisterResolver,
        'LIST_RESOLVERS': handlers.onListResolvers,

        // Partition operations
        'PARTITION_MAP_REQUEST': handlers.onPartitionMapRequest,

        // Full-text search operations
        'SEARCH': handlers.onSearch,
        'SEARCH_SUB': handlers.onSearchSub,
        'SEARCH_UNSUB': handlers.onSearchUnsub,

        // Event journal operations
        'JOURNAL_SUBSCRIBE': handlers.onJournalSubscribe,
        'JOURNAL_UNSUBSCRIBE': handlers.onJournalUnsubscribe,
        'JOURNAL_READ': handlers.onJournalRead,
    };
}
