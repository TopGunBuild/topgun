/**
 * Query Engine - Phase 7 + Phase 8.01 + Phase 8.02
 *
 * CQEngine-inspired query engine for TopGun providing:
 * - O(1) to O(log N) query execution via indexes
 * - Index types: Hash, Navigable, Compound, StandingQuery, Inverted
 * - Cost-based query optimizer
 * - Lazy ResultSet with merge cost optimization
 * - Full CRDT integration (tombstone-aware, TTL-aware)
 * - Full-text search via InvertedIndex (Phase 8.01)
 * - Adaptive indexing with query pattern tracking (Phase 8.02)
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

// Attribute Factory (Phase 9.02)
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
  // FTS Query Types (Phase 12)
  type MatchQueryOptions,
  type MatchQueryNode,
  type MatchPhraseQueryNode,
  type MatchPrefixQueryNode,
  type FTSQueryNode,
  // FTS Plan Types (Phase 12)
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

// Tokenization (Phase 8.01)
export * from './tokenization';

// Adaptive Indexing (Phase 8.02)
export * from './adaptive';

// Query Executor (Phase 12)
export {
  QueryExecutor,
  type StepResult,
  type QueryResult,
  type OrderBy,
  type ExecuteOptions,
} from './QueryExecutor';

// Unified Live Query Registry (Phase 12)
export {
  UnifiedLiveQueryRegistry,
  type UnifiedLiveQueryRegistryOptions,
  type UnifiedLiveQueryRegistryStats,
  type UnifiedDelta,
} from './UnifiedLiveQueryRegistry';
