/**
 * Query Types and Plan Types for Query Engine
 *
 * Defines query node types for the cost-based optimizer
 * and execution plan types.
 *
 * @module query/QueryTypes
 */

import type { Index, IndexQuery } from './indexes/types';

// ============== Query Node Types ==============

/**
 * Base query node interface.
 * Compatible with existing PredicateNode from predicate.ts
 */
export interface QueryNode {
  type: string;
}

/**
 * Simple query node for attribute-based conditions.
 */
export interface SimpleQueryNode extends QueryNode {
  type: 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte' | 'between' | 'like' | 'regex' | 'in' | 'has' | 'contains' | 'containsAll' | 'containsAny';
  attribute: string;
  value?: unknown;
  values?: unknown[];
  /** For 'between' queries: lower bound */
  from?: unknown;
  /** For 'between' queries: upper bound */
  to?: unknown;
  /** For 'between' queries: include lower bound (default: true) */
  fromInclusive?: boolean;
  /** For 'between' queries: include upper bound (default: false) */
  toInclusive?: boolean;
}

// ============== Full-Text Search Query Types ==============

/**
 * Options for full-text search match queries.
 */
export interface MatchQueryOptions {
  /** Minimum BM25 score threshold */
  minScore?: number;
  /** Boost factor for this field */
  boost?: number;
  /** Operator for multi-term queries: 'and' requires all terms, 'or' requires any */
  operator?: 'and' | 'or';
  /** Fuzziness level for typo tolerance (0 = exact, 1 = 1 edit, 2 = 2 edits) */
  fuzziness?: number;
}

/**
 * Match query node for BM25 full-text search.
 */
export interface MatchQueryNode extends QueryNode {
  type: 'match';
  attribute: string;
  query: string;
  options?: MatchQueryOptions;
}

/**
 * Match phrase query node for exact phrase matching.
 */
export interface MatchPhraseQueryNode extends QueryNode {
  type: 'matchPhrase';
  attribute: string;
  query: string;
  /** Word distance tolerance (0 = exact phrase) */
  slop?: number;
}

/**
 * Match prefix query node for prefix matching.
 */
export interface MatchPrefixQueryNode extends QueryNode {
  type: 'matchPrefix';
  attribute: string;
  prefix: string;
  /** Maximum number of term expansions */
  maxExpansions?: number;
}

/**
 * Union type for FTS query nodes.
 */
export type FTSQueryNode = MatchQueryNode | MatchPhraseQueryNode | MatchPrefixQueryNode;

/**
 * Logical query node for combining conditions.
 */
export interface LogicalQueryNode {
  type: 'and' | 'or' | 'not';
  children?: Query[];
  child?: Query;
}

/**
 * Union type for all query types.
 */
export type Query = SimpleQueryNode | LogicalQueryNode | FTSQueryNode;

// ============== Query Options ==============

/**
 * Query execution options for sort/limit/cursor.
 */
export interface QueryOptions {
  /** Sort by field(s): field name -> direction */
  sort?: Record<string, 'asc' | 'desc'>;
  /** Maximum number of results to return */
  limit?: number;
  /** Cursor for pagination (replaces offset) */
  cursor?: string;
}

// ============== Execution Plan Types ==============

/**
 * Point lookup step - O(1) direct key access.
 */
export interface PointLookupStep {
  type: 'point-lookup';
  key: unknown;
  cost: number;
}

/**
 * Multi-point lookup step - O(k) direct key access for k keys.
 */
export interface MultiPointLookupStep {
  type: 'multi-point-lookup';
  keys: unknown[];
  cost: number;
}

/**
 * Execution plan step.
 * Represents a single operation in the query execution plan.
 */
export type PlanStep =
  | PointLookupStep
  | MultiPointLookupStep
  | IndexScanStep
  | FullScanStep
  | IntersectionStep
  | UnionStep
  | FilterStep
  | NotStep
  | FTSScanStep
  | FusionStep;

/**
 * Index scan step - retrieves from an index.
 */
export interface IndexScanStep {
  type: 'index-scan';
  index: Index<unknown, unknown, unknown>;
  query: IndexQuery<unknown>;
}

/**
 * Full scan step - scans all records.
 */
export interface FullScanStep {
  type: 'full-scan';
  predicate: Query;
}

/**
 * Intersection step - AND of multiple result sets.
 */
export interface IntersectionStep {
  type: 'intersection';
  steps: PlanStep[];
}

/**
 * Union step - OR of multiple result sets.
 */
export interface UnionStep {
  type: 'union';
  steps: PlanStep[];
}

/**
 * Filter step - applies predicate to source results.
 */
export interface FilterStep {
  type: 'filter';
  source: PlanStep;
  predicate: Query;
}

/**
 * NOT step - negation (all keys minus matching keys).
 */
export interface NotStep {
  type: 'not';
  source: PlanStep;
  allKeys: () => Set<unknown>;
}

// ============== Full-Text Search Plan Types ==============

/**
 * Fusion strategy for combining results from different search methods.
 */
export type FusionStrategy = 'intersection' | 'rrf' | 'score-filter';

/**
 * FTS scan step - full-text search using FullTextIndex.
 * Returns scored results (documents with BM25 scores).
 */
export interface FTSScanStep {
  type: 'fts-scan';
  /** Field to search */
  field: string;
  /** Search query or phrase */
  query: string;
  /** Type of FTS query */
  ftsType: 'match' | 'matchPhrase' | 'matchPrefix';
  /** Query options (minScore, boost, etc.) */
  options?: MatchQueryOptions;
  /** This step returns scored results */
  returnsScored: true;
  /** Estimated cost */
  estimatedCost: number;
}

/**
 * Fusion step - combines results from multiple steps using RRF or other strategy.
 */
export interface FusionStep {
  type: 'fusion';
  /** Steps to combine */
  steps: PlanStep[];
  /** Fusion strategy */
  strategy: FusionStrategy;
  /** Whether result is scored (true if any child is scored) */
  returnsScored: boolean;
}

// ============== Query Plan ==============

/**
 * Complete query execution plan.
 */
export interface QueryPlan {
  /** Root execution step */
  root: PlanStep;
  /** Estimated execution cost */
  estimatedCost: number;
  /** Whether any index is used */
  usesIndexes: boolean;
  /** Whether sort can use index order */
  indexedSort?: boolean;
  /** Sort configuration */
  sort?: {
    field: string;
    direction: 'asc' | 'desc';
  };
  /** Limit configuration */
  limit?: number;
  /** Cursor for pagination (replaces offset) */
  cursor?: string;
}

// ============== Distributed Cost Model ==============

/**
 * Query execution context for distributed cost estimation.
 */
export interface QueryContext {
  /** Whether query executes in distributed mode */
  isDistributed: boolean;
  /** Number of nodes in cluster */
  nodeCount: number;
  /** Whether query uses PostgreSQL storage */
  usesStorage: boolean;
  /** Local node ID for partition ownership checks */
  localNodeId?: string;
  /** Partition ownership map: partitionId -> ownerNodeId */
  partitionOwners?: Map<number, string>;
}

/**
 * Distributed query cost model.
 * Inspired by Hazelcast CostUtils.java
 */
export interface DistributedCost {
  /** Estimated number of rows */
  rows: number;
  /** CPU cost (computation) */
  cpu: number;
  /** Network cost (data transfer between nodes) */
  network: number;
  /** I/O cost (disk reads for PostgreSQL) */
  io: number;
}

/**
 * Cost multipliers for distributed query optimization.
 * Network is weighted 10x higher than CPU because network latency
 * typically dominates query execution time in distributed systems.
 */
export const COST_WEIGHTS = {
  CPU: 1.0,
  NETWORK: 10.0,    // Network is expensive (latency, bandwidth)
  IO: 5.0,          // Disk I/O is moderately expensive
  ROWS: 0.001,      // Row count factor
} as const;

/**
 * Calculate total cost from distributed cost components.
 *
 * @param cost - Distributed cost breakdown
 * @returns Weighted total cost
 */
export function calculateTotalCost(cost: DistributedCost): number {
  return (
    cost.rows * COST_WEIGHTS.ROWS +
    cost.cpu * COST_WEIGHTS.CPU +
    cost.network * COST_WEIGHTS.NETWORK +
    cost.io * COST_WEIGHTS.IO
  );
}

// ============== Type Guards ==============

/**
 * Check if a query is a simple query node.
 */
export function isSimpleQuery(query: Query): query is SimpleQueryNode {
  return [
    'eq',
    'neq',
    'gt',
    'gte',
    'lt',
    'lte',
    'between',
    'like',
    'regex',
    'in',
    'has',
    'contains',
    'containsAll',
    'containsAny',
  ].includes(query.type);
}

/**
 * Check if a query is a logical query node.
 */
export function isLogicalQuery(query: Query): query is LogicalQueryNode {
  return query.type === 'and' || query.type === 'or' || query.type === 'not';
}

/**
 * Check if a query is a full-text search query node.
 */
export function isFTSQuery(query: Query): query is FTSQueryNode {
  return query.type === 'match' || query.type === 'matchPhrase' || query.type === 'matchPrefix';
}

/**
 * Check if a query is a match query node.
 */
export function isMatchQuery(query: Query): query is MatchQueryNode {
  return query.type === 'match';
}

/**
 * Check if a query is a match phrase query node.
 */
export function isMatchPhraseQuery(query: Query): query is MatchPhraseQueryNode {
  return query.type === 'matchPhrase';
}

/**
 * Check if a query is a match prefix query node.
 */
export function isMatchPrefixQuery(query: Query): query is MatchPrefixQueryNode {
  return query.type === 'matchPrefix';
}
