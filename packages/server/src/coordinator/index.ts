// Coordinator modules - extracted from ServerCoordinator
// These modules handle focused responsibilities with clear interfaces

export { AuthHandler } from './auth-handler';
export { ConnectionManager } from './connection-manager';
export { StorageManager } from './storage-manager';
export { OperationHandler } from './operation-handler';
export { createMessageRegistry } from './message-registry';

// SPEC-003a: Broadcast handler
export { BroadcastHandler } from './broadcast-handler';

// SPEC-003b: GC handler
export { GCHandler } from './gc-handler';

// SPEC-003c: Cluster event handler
export { ClusterEventHandler } from './cluster-event-handler';

// SPEC-003d: Additional handlers
export { HeartbeatHandler } from './heartbeat-handler';
export { ClientMessageHandler } from './client-message-handler';
export { PersistenceHandler } from './persistence-handler';
export { OperationContextHandler } from './operation-context-handler';
export { QueryConversionHandler } from './query-conversion-handler';
export { BatchProcessingHandler } from './batch-processing-handler';
export { WriteConcernHandler } from './write-concern-handler';

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

// Phase 1: WebSocket handler
export { WebSocketHandler } from './websocket-handler';

// Phase 2: Lifecycle manager
export { LifecycleManager } from './lifecycle-manager';

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
    // Broadcast types
    IBroadcastHandler,
    BroadcastHandlerConfig,
    // GC types
    IGCHandler,
    GCHandlerConfig,
    // Cluster event types
    IClusterEventHandler,
    ClusterEventHandlerConfig,
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
    // Heartbeat types (SPEC-003d)
    IHeartbeatHandler,
    HeartbeatHandlerConfig,
    // Client message types (SPEC-003d)
    IClientMessageHandler,
    ClientMessageHandlerConfig,
    // Persistence types (SPEC-003d)
    IPersistenceHandler,
    PersistenceHandlerConfig,
    // Operation context types (SPEC-003d)
    IOperationContextHandler,
    OperationContextHandlerConfig,
    // Query conversion types (SPEC-003d)
    IQueryConversionHandler,
    QueryConversionHandlerConfig,
    // Batch processing types (SPEC-003d)
    IBatchProcessingHandler,
    BatchProcessingHandlerConfig,
    // Write concern types (SPEC-003d)
    IWriteConcernHandler,
    WriteConcernHandlerConfig,
    // WebSocket types (Phase 1)
    IWebSocketHandler,
    WebSocketHandlerConfig,
} from './types';
