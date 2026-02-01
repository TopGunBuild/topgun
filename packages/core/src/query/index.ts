/**
 * Query Engine
 *
 * CQEngine-inspired query engine for TopGun providing:
 * - O(1) to O(log N) query execution via indexes
 * - Index types: Hash, Navigable, Compound, StandingQuery, Inverted
 * - Cost-based query optimizer
 * - Lazy ResultSet with merge cost optimization
 * - Full CRDT integration (tombstone-aware, TTL-aware)
 * - Full-text search via InvertedIndex
 * - Adaptive indexing with query pattern tracking
 */

// Data structures
export * from './ds';

// Attribute system
export {
  type Attribute,
  SimpleAttribute,
  MultiValueAttribute,
  simpleAttribute,
  multiAttribute,
} from './Attribute';

// Attribute Factory
export {
  generateAttributes,
  attr,
  multiAttr,
  createSchema,
  type AttributeType,
  type AttributeSchema,
  type GeneratedAttributes,
  type GenerateAttributesOptions,
} from './AttributeFactory';

// Indexes
export * from './indexes';

// ResultSet
export * from './resultset';

// Query Types
export {
  type QueryNode,
  type SimpleQueryNode,
  type LogicalQueryNode,
  type Query,
  type QueryOptions,
  type PlanStep,
  type IndexScanStep,
  type FullScanStep,
  type IntersectionStep,
  type UnionStep,
  type FilterStep,
  type NotStep,
  type QueryPlan,
  // FTS Query Types
  type MatchQueryOptions,
  type MatchQueryNode,
  type MatchPhraseQueryNode,
  type MatchPrefixQueryNode,
  type FTSQueryNode,
  // FTS Plan Types
  type FTSScanStep,
  type FusionStep,
  type FusionStrategy,
  // Type guards
  isSimpleQuery,
  isLogicalQuery,
  isFTSQuery,
  isMatchQuery,
  isMatchPhraseQuery,
  isMatchPrefixQuery,
} from './QueryTypes';

// Index Registry
export { IndexRegistry, type IndexRegistryStats } from './IndexRegistry';

// Query Optimizer
export {
  QueryOptimizer,
  type QueryOptimizerOptions,
  type ClassifiedPredicates,
} from './QueryOptimizer';

// Standing Query Registry
export {
  StandingQueryRegistry,
  type StandingQueryRegistryOptions,
  type StandingQueryRegistryStats,
} from './StandingQueryRegistry';

// Live Query Manager
export {
  LiveQueryManager,
  type LiveQueryManagerOptions,
  type LiveQueryManagerStats,
  type LiveQueryCallback,
  type LiveQueryEvent,
  type LiveQueryInitialEvent,
  type LiveQueryDeltaEvent,
} from './LiveQueryManager';

// Tokenization
export * from './tokenization';

// Adaptive Indexing
export * from './adaptive';

// Query Executor
export {
  QueryExecutor,
  type StepResult,
  type QueryResult,
  type OrderBy,
  type ExecuteOptions,
  type QueryResultWithCursor,
  type CursorStatus,
} from './QueryExecutor';

// Unified Live Query Registry
export {
  UnifiedLiveQueryRegistry,
  type UnifiedLiveQueryRegistryOptions,
  type UnifiedLiveQueryRegistryStats,
  type UnifiedDelta,
} from './UnifiedLiveQueryRegistry';

// Query Cursor
export {
  QueryCursor,
  type QueryCursorData,
  type CursorableQueryResult,
  type QueryCursorOptions,
  DEFAULT_QUERY_CURSOR_MAX_AGE_MS,
} from './QueryCursor';
