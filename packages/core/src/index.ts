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

// Event Journal exports
export { Ringbuffer, EventJournalImpl, DEFAULT_EVENT_JOURNAL_CONFIG };
export type {
  EventJournal,
  EventJournalConfig,
  JournalEvent,
  JournalEventInput,
  JournalEventType,
  JournalEventListener,
};

// Entry Processor exports
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

// Conflict Resolver exports
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

// Write Concern exports
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

// Cluster types exports
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
  // Migration types
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
  // Replication types
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
  // Migration exports
  PartitionState,
  DEFAULT_MIGRATION_CONFIG,
  // Replication exports
  ConsistencyLevel,
  DEFAULT_REPLICATION_CONFIG,
} from './types/cluster';

// Query Engine exports
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
  // Tokenization
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
  // Distributed Cost Model
  COST_WEIGHTS,
  calculateTotalCost,
  // Standing Query Registry
  StandingQueryRegistry,
  // Live Query Manager
  LiveQueryManager,
  // Query Cursor
  QueryCursor,
  DEFAULT_QUERY_CURSOR_MAX_AGE_MS,
} from './query';

// Base64URL utilities
export { encodeBase64Url, decodeBase64Url } from './utils/base64url';

// Compare utilities
export { compareValues } from './utils/compare';

// Logger utilities
export { logger } from './utils/logger';
export type { Logger } from './utils/logger';

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
  // Tokenization
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
  // Distributed Cost Model
  QueryContext,
  DistributedCost,
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
  // Query Cursor
  QueryCursorData,
  CursorableQueryResult,
  QueryCursorOptions,
  QueryResultWithCursor,
} from './query';

// Re-export Query from query module as QueryExpression to avoid conflict
export type { Query as QueryExpression } from './query';

// Indexed CRDT exports
export { IndexedLWWMap } from './IndexedLWWMap';
export { IndexedORMap, type ORMapQueryResult, type ORMapSearchResult } from './IndexedORMap';

// Full-Text Search exports
export {
  // Tokenizer
  Tokenizer as FTSTokenizer,
  ENGLISH_STOPWORDS,
  porterStem,
  // Inverted Index
  InvertedIndex as FTSInvertedIndex,
  // BM25 Scorer
  BM25Scorer,
  // Full-Text Index (high-level integration)
  FullTextIndex,
} from './fts';
export type {
  // Types
  TokenizerOptions as FTSTokenizerOptions,
  TermInfo,
  Posting,
  BM25Options,
  ScoredDocument,
  FullTextIndexConfig,
  SearchOptions as FTSSearchOptions,
  SearchResult as FTSSearchResult,
  SerializedIndex,
} from './fts';

// Search utilities exports
export {
  ReciprocalRankFusion,
  SearchCursor,
  DEFAULT_CURSOR_MAX_AGE_MS,
} from './search';
export type {
  RankedResult,
  RRFConfig,
  MergedResult,
  SearchCursorData,
  CursorableResult,
} from './search';

// Debug utilities exports
export {
  CRDTDebugger,
  getCRDTDebugger,
  resetCRDTDebugger,
  SearchDebugger,
  getSearchDebugger,
  resetSearchDebugger,
} from './debug';
export type {
  CRDTSnapshot,
  ConflictInfo,
  DebugStatistics,
  OperationQueryOptions,
  BM25DebugInfo,
  ExactMatchDebugInfo,
  RRFDebugInfo,
  VectorDebugInfo,
  SearchResultDebug,
  SearchIndexStats,
  SearchTiming,
  SearchDebugInfo,
} from './debug';

// Deterministic Simulation Testing (DST) exports
export {
  VirtualClock,
  RealClock,
  SeededRNG,
  VirtualNetwork,
  InvariantChecker,
  CRDTInvariants,
  ScenarioRunner,
} from './testing';
export type {
  ClockSource,
  NetworkConfig,
  Message,
  Invariant,
  InvariantResult,
  ScenarioConfig,
  ScenarioResult,
} from './testing';
