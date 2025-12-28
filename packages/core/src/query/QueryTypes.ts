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
  type: 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte' | 'between' | 'like' | 'regex' | 'in' | 'has';
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
export type Query = SimpleQueryNode | LogicalQueryNode;

// ============== Query Options ==============

/**
 * Query execution options for sort/limit/offset.
 */
export interface QueryOptions {
  /** Sort by field(s): field name -> direction */
  sort?: Record<string, 'asc' | 'desc'>;
  /** Maximum number of results to return */
  limit?: number;
  /** Number of results to skip */
  offset?: number;
}

// ============== Execution Plan Types ==============

/**
 * Execution plan step.
 * Represents a single operation in the query execution plan.
 */
export type PlanStep =
  | IndexScanStep
  | FullScanStep
  | IntersectionStep
  | UnionStep
  | FilterStep
  | NotStep;

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
  /** Offset configuration */
  offset?: number;
}

// ============== Type Guards ==============

/**
 * Check if a query is a simple query node.
 */
export function isSimpleQuery(query: Query): query is SimpleQueryNode {
  return ['eq', 'neq', 'gt', 'gte', 'lt', 'lte', 'between', 'like', 'regex', 'in', 'has'].includes(
    query.type
  );
}

/**
 * Check if a query is a logical query node.
 */
export function isLogicalQuery(query: Query): query is LogicalQueryNode {
  return query.type === 'and' || query.type === 'or' || query.type === 'not';
}
