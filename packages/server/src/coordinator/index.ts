// Coordinator modules - extracted from ServerCoordinator
// These modules handle focused responsibilities with clear interfaces

export { AuthHandler } from './auth-handler';
export { ConnectionManager } from './connection-manager';
export { StorageManager } from './storage-manager';
export { OperationHandler } from './operation-handler';
export { createMessageRegistry } from './message-registry';

// Phase 4: Additional handler modules
export { PartitionHandler } from './partition-handler';
export { TopicHandler } from './topic-handler';
export { LockHandler } from './lock-handler';
export { CounterHandlerAdapter } from './counter-handler-adapter';
export { ResolverHandler } from './resolver-handler';
export { JournalHandler } from './journal-handler';
export { LwwSyncHandler } from './lww-sync-handler';
export { ORMapSyncHandler } from './ormap-sync-handler';
export { EntryProcessorAdapter } from './entry-processor-adapter';
export { SearchHandler } from './search-handler';
export { QueryHandler } from './query-handler';

export type { MessageRegistry, MessageHandler, MessageHandlers } from './message-registry';
export type {
    // Auth types
    IAuthHandler,
    AuthHandlerConfig,
    AuthResult,
    // Connection types
    ClientConnection,
    IConnectionManager,
    ConnectionManagerConfig,
    // Storage types
    IStorageManager,
    StorageManagerConfig,
    // Operation types
    IOperationHandler,
    OperationHandlerConfig,
    // Partition types
    IPartitionHandler,
    PartitionHandlerConfig,
    // Topic types
    ITopicHandler,
    TopicHandlerConfig,
    // Lock types
    ILockHandler,
    LockHandlerConfig,
    // Counter types
    ICounterHandlerAdapter,
    CounterHandlerAdapterConfig,
    // Resolver types
    IResolverHandler,
    ResolverHandlerConfig,
    // Journal types
    IJournalHandler,
    JournalHandlerConfig,
    // LWW Sync types
    ILwwSyncHandler,
    LwwSyncHandlerConfig,
    // ORMap Sync types
    IORMapSyncHandler,
    ORMapSyncHandlerConfig,
    // Entry Processor types
    IEntryProcessorAdapter,
    EntryProcessorAdapterConfig,
    // Search types
    ISearchHandler,
    SearchHandlerConfig,
    // Query types
    IQueryHandler,
    QueryHandlerConfig,
} from './types';
