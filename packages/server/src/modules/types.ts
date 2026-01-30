import type { Server as HttpServer } from 'node:http';
import type { Server as HttpsServer } from 'node:https';
import type { WebSocketServer } from 'ws';
import type { HLC, PermissionPolicy, ConsistencyLevel, ReplicationConfig, FullTextIndexConfig } from '@topgunbuild/core';
import type { MetricsService } from '../monitoring/MetricsService';
import type { SecurityManager } from '../security/SecurityManager';
import type { StripedEventExecutor } from '../utils/StripedEventExecutor';
import type { BackpressureRegulator } from '../utils/BackpressureRegulator';
import type { ConnectionRateLimiter } from '../utils/ConnectionRateLimiter';
import type { RateLimitedLogger } from '../utils/RateLimitedLogger';
import type { WorkerPool, MerkleWorker, CRDTMergeWorker, SerializationWorker, WorkerPoolConfig } from '../workers';
import type { ClusterManager } from '../cluster/ClusterManager';
import type { PartitionService } from '../cluster/PartitionService';
import type { ReplicationPipeline } from '../cluster/ReplicationPipeline';
import type { LockManager } from '../cluster/LockManager';
import type { MerkleTreeManager } from '../cluster/MerkleTreeManager';
import type { PartitionReassigner } from '../cluster/PartitionReassigner';
import type { ReadReplicaHandler } from '../cluster/ReadReplicaHandler';
import type { RepairScheduler } from '../cluster/RepairScheduler';
import type { TLSConfig, ClusterTLSConfig } from '../types/TLSConfig';
import type { StorageManager } from '../coordinator/storage-manager';
import type { QueryRegistry } from '../query/QueryRegistry';
import type { ObjectPool, PooledEventPayload } from '../memory';
import type { TaskletScheduler } from '../tasklet';
import type { WriteAckManager } from '../ack/WriteAckManager';
import type { IServerStorage } from '../storage';
import type {
  AuthHandler,
  OperationHandler,
  BatchProcessingHandler,
  GCHandler,
  LwwSyncHandler,
  ORMapSyncHandler,
  QueryHandler,
  QueryConversionHandler,
  TopicHandler,
  BroadcastHandler,
  LockHandler,
  PartitionHandler,
  SearchHandler,
  JournalHandler,
  CounterHandlerAdapter,
  EntryProcessorAdapter,
  ResolverHandler,
  WebSocketHandler,
  ClientMessageHandler,
  HeartbeatHandler,
  PersistenceHandler,
  OperationContextHandler,
  WriteConcernHandler,
} from '../coordinator';
import type { MessageRegistry } from '../coordinator/message-registry';
import type { CounterHandler } from '../handlers/CounterHandler';
import type { EntryProcessorHandler } from '../handlers/EntryProcessorHandler';
import type { ConflictResolverHandler } from '../handlers/ConflictResolverHandler';
import type { TopicManager } from '../topic/TopicManager';
import type { SearchCoordinator, ClusterSearchCoordinator } from '../search';
import type { DistributedSubscriptionCoordinator } from '../subscriptions/DistributedSubscriptionCoordinator';
import type { ConnectionManager } from '../coordinator/connection-manager';
import type { EventJournalService } from '../EventJournalService';
import type { CoalescingWriterOptions } from '../utils/CoalescingWriter';
import type { IInterceptor } from '../interceptor/IInterceptor';
import type { ClusterSearchConfig } from '../search/ClusterSearchCoordinator';

// Re-export for convenience
export type FTSConfig = FullTextIndexConfig;
export type DistributedSearchConfig = ClusterSearchConfig;

// Core module - no dependencies
export interface CoreModule {
  hlc: HLC;
  metricsService: MetricsService;
  securityManager: SecurityManager;
  eventExecutor: StripedEventExecutor;
  backpressure: BackpressureRegulator;
}

export interface CoreModuleConfig {
  nodeId: string;
  eventStripeCount?: number;
  eventQueueCapacity?: number;
  backpressureEnabled?: boolean;
  backpressureSyncFrequency?: number;
  backpressureMaxPending?: number;
  backpressureBackoffMs?: number;
  securityPolicies?: PermissionPolicy[];
}

// Worker module - optional, depends on config
export interface WorkerModule {
  workerPool?: WorkerPool;
  merkleWorker?: MerkleWorker;
  crdtMergeWorker?: CRDTMergeWorker;
  serializationWorker?: SerializationWorker;
}

export interface WorkerModuleConfig {
  workerPoolEnabled?: boolean;
  workerPoolConfig?: Partial<WorkerPoolConfig>;
}

// Cluster module - depends on core
export interface ClusterModuleConfig {
  nodeId: string;
  host?: string;
  clusterPort?: number;
  peers?: string[];
  resolvePeers?: () => string[];
  discovery?: 'manual' | 'kubernetes';
  serviceName?: string;
  discoveryInterval?: number;
  clusterTls?: ClusterTLSConfig;
  replicationEnabled?: boolean;
  defaultConsistency?: ConsistencyLevel;
  replicationConfig?: Partial<ReplicationConfig>;
}

export interface ClusterModuleDeps {
  hlc: HLC;
}

export interface ClusterModule {
  cluster: ClusterManager;
  partitionService: PartitionService;
  replicationPipeline?: ReplicationPipeline;
  lockManager: LockManager;
  merkleTreeManager: MerkleTreeManager;
  partitionReassigner: PartitionReassigner;
  readReplicaHandler: ReadReplicaHandler;
  repairScheduler: RepairScheduler;
}

// Storage module - depends on core and cluster
export interface StorageModuleConfig {
  nodeId: string;
  storage?: IServerStorage;
  fullTextSearch?: Record<string, FullTextIndexConfig>;
  writeAckTimeout?: number;
}

export interface StorageModuleDeps {
  hlc: HLC;
  metricsService: MetricsService;
  partitionService: PartitionService;
}

export interface StorageModule {
  storageManager: StorageManager;
  queryRegistry: QueryRegistry;
  eventPayloadPool: ObjectPool<PooledEventPayload>;
  taskletScheduler: TaskletScheduler;
  writeAckManager: WriteAckManager;
}

// Network module - HTTP, WSS, rate limiting
export interface NetworkModuleConfig {
  port: number;
  tls?: TLSConfig;
  wsBacklog?: number;
  wsCompression?: boolean;
  wsMaxPayload?: number;
  maxConnections?: number;
  serverTimeout?: number;
  keepAliveTimeout?: number;
  headersTimeout?: number;
  maxConnectionsPerSecond?: number;
  maxPendingConnections?: number;
  // Socket-level configuration
  socketNoDelay?: boolean;       // Default: true
  socketKeepAlive?: boolean;     // Default: true
  socketKeepAliveMs?: number;    // Default: 60000
}

export interface NetworkModuleDeps {
  // Currently no dependencies required for HTTP/WSS/RateLimiter creation
  // metricsService could be added in future for connection tracking metrics
}

export interface NetworkModule {
  httpServer: HttpServer | HttpsServer;
  wss: WebSocketServer;
  rateLimiter: ConnectionRateLimiter;
  rateLimitedLogger: RateLimitedLogger;
  start: () => void;  // DEFERRED startup - call AFTER assembly
}

// CRDT handlers - conflict resolution and operations
export interface CRDTHandlers {
  operationHandler: OperationHandler;
  batchProcessingHandler: BatchProcessingHandler;
  gcHandler: GCHandler;
}

// Sync handlers - merkle tree and OR-Map sync
export interface SyncHandlers {
  lwwSyncHandler: LwwSyncHandler;
  orMapSyncHandler: ORMapSyncHandler;
}

// Query handlers - subscriptions and conversions
export interface QueryHandlers {
  queryHandler: QueryHandler;
  queryConversionHandler: QueryConversionHandler;
}

// Messaging handlers - topics, broadcast
export interface MessagingHandlers {
  topicHandler: TopicHandler;
  broadcastHandler: BroadcastHandler;
}

// Coordination handlers - locks, partitions
export interface CoordinationHandlers {
  lockHandler: LockHandler;
  partitionHandler: PartitionHandler;
}

// Search handlers
export interface SearchHandlers {
  searchHandler: SearchHandler;
}

// Persistence handlers - journals, counters, entry processors
export interface PersistenceHandlers {
  journalHandler: JournalHandler;
  counterHandler: CounterHandlerAdapter;
  entryProcessorHandler: EntryProcessorAdapter;
  resolverHandler: ResolverHandler;
}

// Client handlers - auth, websocket, client messages
export interface ClientHandlers {
  authHandler: AuthHandler;
  webSocketHandler: WebSocketHandler;
  clientMessageHandler: ClientMessageHandler;
}

// Server handlers - heartbeat, persistence, write concern
export interface ServerHandlers {
  heartbeatHandler: HeartbeatHandler;
  persistenceHandler: PersistenceHandler;
  operationContextHandler: OperationContextHandler;
  writeConcernHandler: WriteConcernHandler;
}

// All handlers combined (ClusterHandlers excluded - not instantiated in current code)
export interface HandlersModule {
  crdt: CRDTHandlers;
  sync: SyncHandlers;
  query: QueryHandlers;
  messaging: MessagingHandlers;
  coordination: CoordinationHandlers;
  search: SearchHandlers;
  persistence: PersistenceHandlers;
  client: ClientHandlers;
  server: ServerHandlers;
  messageRegistry: MessageRegistry;
  // Internal managers created by handlers-module (not exposed)
  _internal: {
    topicManager: TopicManager;
    searchCoordinator: SearchCoordinator;
    clusterSearchCoordinator: ClusterSearchCoordinator;
    distributedSubCoordinator: DistributedSubscriptionCoordinator;
    connectionManager: ConnectionManager;
    counterHandler: CounterHandler;
    entryProcessorHandler: EntryProcessorHandler;
    conflictResolverHandler: ConflictResolverHandler;
    eventJournalService?: EventJournalService;
    pendingClusterQueries: Map<string, any>;
    pendingBatchOperations: Set<Promise<void>>;
    journalSubscriptions: Map<string, any>;
  };
}

export interface HandlersModuleConfig {
  nodeId: string;
  jwtSecret: string;
  rateLimitingEnabled?: boolean;
  writeCoalescingEnabled?: boolean;
  writeCoalescingOptions?: Partial<CoalescingWriterOptions>;
  interceptors?: IInterceptor[];
  storage?: IServerStorage;
  eventJournalEnabled?: boolean;
  eventJournalConfig?: Partial<any>;
  fullTextSearch?: Record<string, FTSConfig>;
  distributedSearch?: DistributedSearchConfig;
  defaultConsistency?: ConsistencyLevel;
}

// Dependencies from other modules
export interface HandlersModuleDeps {
  // From CoreModule
  core: {
    hlc: HLC;
    metricsService: MetricsService;
    securityManager: SecurityManager;
    eventExecutor: StripedEventExecutor;
    backpressure: BackpressureRegulator;
  };
  // From NetworkModule
  network: {
    rateLimiter: ConnectionRateLimiter;
    rateLimitedLogger: RateLimitedLogger;
  };
  // From ClusterModule
  cluster: {
    cluster: ClusterManager;
    partitionService: PartitionService;
    replicationPipeline?: ReplicationPipeline;
    lockManager: LockManager;
    merkleTreeManager?: MerkleTreeManager;
    readReplicaHandler?: ReadReplicaHandler;
  };
  // From StorageModule
  storage: {
    storageManager: StorageManager;
    queryRegistry: QueryRegistry;
    writeAckManager: WriteAckManager;
  };
}

// Placeholder interfaces for later sub-specs
export interface SearchModule { /* defined in SPEC-011e */ }
export interface LifecycleModule { /* defined in SPEC-011e */ }

// All modules combined
export interface ServerModules {
  core: CoreModule;
  network: NetworkModule;
  cluster: ClusterModule;
  storage: StorageModule;
  workers: WorkerModule;
  handlers: HandlersModule;
  search: SearchModule;
  lifecycle: LifecycleModule;
}
