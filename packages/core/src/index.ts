import { HLC, Timestamp } from './HLC';
import { LWWMap, LWWRecord } from './LWWMap';
import { ORMap, ORMapRecord, MergeKeyResult, ORMapSnapshot } from './ORMap';
import { MerkleTree } from './MerkleTree';
import { ORMapMerkleTree, ORMapMerkleNode } from './ORMapMerkleTree';
import { hashORMapEntry, hashORMapRecord, timestampToString, compareTimestamps } from './ORMapMerkle';
import { PNCounterImpl } from './PNCounter';
import type { PNCounter, PNCounterState, PNCounterStateObject, PNCounterConfig } from './PNCounter';
import { Ringbuffer } from './Ringbuffer';
import { EventJournalImpl, DEFAULT_EVENT_JOURNAL_CONFIG } from './EventJournal';
import type {
  EventJournal,
  EventJournalConfig,
  JournalEvent,
  JournalEventInput,
  JournalEventType,
  JournalEventListener,
} from './EventJournal';

export { HLC, LWWMap, ORMap, MerkleTree, ORMapMerkleTree, PNCounterImpl };
export { hashORMapEntry, hashORMapRecord, timestampToString, compareTimestamps };
export type { PNCounter, PNCounterState, PNCounterStateObject, PNCounterConfig };

// Event Journal exports (Phase 5.04)
export { Ringbuffer, EventJournalImpl, DEFAULT_EVENT_JOURNAL_CONFIG };
export type {
  EventJournal,
  EventJournalConfig,
  JournalEvent,
  JournalEventInput,
  JournalEventType,
  JournalEventListener,
};

// Entry Processor exports (Phase 5.03)
export {
  EntryProcessorDefSchema,
  validateProcessorCode,
  BuiltInProcessors,
  FORBIDDEN_PATTERNS,
  DEFAULT_PROCESSOR_RATE_LIMITS,
} from './EntryProcessor';
export type {
  EntryProcessorFn,
  EntryProcessorDef,
  EntryProcessorResult,
  ProcessorRateLimitConfig,
} from './EntryProcessor';

// Conflict Resolver exports (Phase 5.05)
export {
  ConflictResolverDefSchema,
  validateResolverCode,
  BuiltInResolvers,
  RESOLVER_FORBIDDEN_PATTERNS,
  DEFAULT_RESOLVER_RATE_LIMITS,
  compareHLCTimestamps,
  deepMerge,
} from './ConflictResolver';
export type {
  MergeContext,
  MergeResult,
  ConflictResolverFn,
  ConflictResolverDef,
  ResolverRateLimitConfig,
  MergeRejection,
} from './ConflictResolver';

export * from './utils/hash';
export * from './serializer';
export * from './predicate';
export * from './security';
export * from './schemas';
export type { Timestamp, LWWRecord, ORMapRecord, MergeKeyResult, ORMapSnapshot, ORMapMerkleNode };

// Re-export heartbeat types for convenience
export type { PingMessage, PongMessage } from './schemas';

// Write Concern exports (Phase 5.01)
export {
  WriteConcern,
  WriteOptions,
  WriteResult,
  PendingWrite,
  WRITE_CONCERN_ORDER,
  DEFAULT_WRITE_CONCERN_TIMEOUT,
  isWriteConcernAchieved,
  getHighestWriteConcernLevel,
} from './types/WriteConcern';
export type { WriteConcernValue } from './schemas';

// Cluster types exports (Phase 4)
export type {
  NodeStatus,
  NodeInfo,
  PartitionInfo,
  PartitionMap,
  PartitionMapMessage,
  PartitionMapRequestMessage,
  PartitionChange,
  PartitionMapDeltaMessage,
  NotOwnerError,
  StaleMapError,
  RoutingError,
  ConnectionPoolConfig,
  PartitionRouterConfig,
  ClusterClientConfig,
  CircuitBreakerConfig,
  ConnectionState,
  NodeHealth,
  ClusterEvents,
  // Migration types (Task 03)
  PartitionMigration,
  MigrationConfig,
  MigrationStatus,
  MigrationMetrics,
  MigrationStartMessage,
  MigrationChunkMessage,
  MigrationChunkAckMessage,
  MigrationCompleteMessage,
  MigrationVerifyMessage,
  MigrationMessage,
  // Replication types (Task 04)
  WriteOptions as ClusterWriteOptions,
  ReadOptions as ClusterReadOptions,
  ReplicationConfig,
  ReplicationTask,
  ReplicationLag,
  ReplicationHealth,
  ReplicationResult,
  ReplicationMessage,
  ReplicationBatchMessage,
  ReplicationAckMessage,
  ReplicationBatchAckMessage,
  ReplicationProtocolMessage,
} from './types/cluster';
export {
  DEFAULT_CONNECTION_POOL_CONFIG,
  DEFAULT_PARTITION_ROUTER_CONFIG,
  DEFAULT_CIRCUIT_BREAKER_CONFIG,
  PARTITION_COUNT,
  DEFAULT_BACKUP_COUNT,
  // Migration exports (Task 03)
  PartitionState,
  DEFAULT_MIGRATION_CONFIG,
  // Replication exports (Task 04)
  ConsistencyLevel,
  DEFAULT_REPLICATION_CONFIG,
} from './types/cluster';

// Query Engine exports (Phase 7 + Phase 8.01)
// Note: Query from schemas conflicts with Query from query/QueryTypes
// We rename query engine Query to QueryExpression to avoid conflict
export {
  // Data structures
  SortedMap,
  // Attribute system
  SimpleAttribute,
  MultiValueAttribute,
  simpleAttribute,
  multiAttribute,
  // Indexes
  HashIndex,
  NavigableIndex,
  StandingQueryIndex,
  FallbackIndex,
  createPredicateMatcher,
  InvertedIndex,
  // Tokenization (Phase 8.01)
  TokenizationPipeline,
  WhitespaceTokenizer,
  WordBoundaryTokenizer,
  NGramTokenizer,
  LowercaseFilter,
  StopWordFilter,
  MinLengthFilter,
  MaxLengthFilter,
  TrimFilter,
  UniqueFilter,
  DEFAULT_STOP_WORDS,
  // ResultSet
  SetResultSet,
  LazyResultSet,
  IntersectionResultSet,
  UnionResultSet,
  FilteringResultSet,
  SortedResultSet,
  createFieldComparator,
  LimitResultSet,
  // Type Guards
  isSimpleQuery,
  isLogicalQuery,
  // Index Registry
  IndexRegistry,
  // Query Optimizer
  QueryOptimizer,
  // Standing Query Registry
  StandingQueryRegistry,
  // Live Query Manager
  LiveQueryManager,
} from './query';

export type {
  // Attribute system
  Attribute,
  // Indexes
  StandingQueryChange,
  StandingQueryIndexOptions,
  Index,
  IndexQuery,
  IndexStats,
  InvertedIndexStats,
  // Tokenization (Phase 8.01)
  Tokenizer,
  TokenFilter,
  TokenizationPipelineOptions,
  // ResultSet
  ResultSet,
  IteratorFactory,
  PredicateFn,
  CompareFn,
  // Query Types (renamed to avoid conflict with schemas.Query)
  QueryNode,
  SimpleQueryNode,
  LogicalQueryNode,
  QueryOptions,
  PlanStep,
  IndexScanStep,
  FullScanStep,
  IntersectionStep,
  UnionStep,
  FilterStep,
  NotStep,
  QueryPlan,
  // Index Registry
  IndexRegistryStats,
  // Query Optimizer
  QueryOptimizerOptions,
  // Standing Query Registry
  StandingQueryRegistryOptions,
  StandingQueryRegistryStats,
  // Live Query Manager
  LiveQueryManagerOptions,
  LiveQueryManagerStats,
  LiveQueryCallback,
  LiveQueryEvent,
  LiveQueryInitialEvent,
  LiveQueryDeltaEvent,
} from './query';

// Re-export Query from query module as QueryExpression to avoid conflict
export type { Query as QueryExpression } from './query';

// Indexed CRDT exports (Phase 7.07)
export { IndexedLWWMap } from './IndexedLWWMap';
export { IndexedORMap, type ORMapQueryResult } from './IndexedORMap';
