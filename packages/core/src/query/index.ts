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
  isSimpleQuery,
  isLogicalQuery,
} from './QueryTypes';

// Index Registry
export { IndexRegistry, type IndexRegistryStats } from './IndexRegistry';

// Query Optimizer
export { QueryOptimizer, type QueryOptimizerOptions } from './QueryOptimizer';

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
