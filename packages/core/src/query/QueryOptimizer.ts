/**
 * QueryOptimizer Implementation
 *
 * Cost-based query optimizer for the Query Engine.
 * Selects optimal index and execution strategy for queries.
 *
 * Algorithm based on CQEngine CollectionQueryEngine:
 * - AND queries: "smallest first" strategy - sort by merge cost, iterate smallest
 * - OR queries: Union all results with deduplication
 * - NOT queries: Get all keys, subtract matching keys
 *
 * @module query/QueryOptimizer
 */

import { IndexRegistry } from './IndexRegistry';
import type {
  Query,
  SimpleQueryNode,
  LogicalQueryNode,
  QueryPlan,
  PlanStep,
  QueryOptions,
} from './QueryTypes';
import { isLogicalQuery, isSimpleQuery } from './QueryTypes';
import type { IndexQuery } from './indexes/types';

/**
 * Cost-based query optimizer.
 * Selects optimal index and execution strategy for queries.
 *
 * K = record key type, V = record value type
 */
export class QueryOptimizer<K, V> {
  constructor(private readonly indexRegistry: IndexRegistry<K, V>) {}

  /**
   * Optimize a query and return an execution plan.
   *
   * @param query - Query to optimize
   * @returns Query execution plan
   */
  optimize(query: Query): QueryPlan {
    const step = this.optimizeNode(query);
    return {
      root: step,
      estimatedCost: this.estimateCost(step),
      usesIndexes: this.usesIndexes(step),
    };
  }

  /**
   * Optimize a query with sort/limit/offset options.
   *
   * @param query - Query to optimize
   * @param options - Query options (sort, limit, offset)
   * @returns Query execution plan with options
   */
  optimizeWithOptions(query: Query, options: QueryOptions): QueryPlan {
    const basePlan = this.optimize(query);

    // If no options specified, return base plan
    if (!options.sort && options.limit === undefined && options.offset === undefined) {
      return basePlan;
    }

    let indexedSort = false;
    let sortField: string | undefined;
    let sortDirection: 'asc' | 'desc' | undefined;

    // Check if sort can use NavigableIndex
    if (options.sort) {
      const sortFields = Object.keys(options.sort);
      if (sortFields.length > 0) {
        sortField = sortFields[0];
        sortDirection = options.sort[sortField];

        // Look for a NavigableIndex on the sort field
        const sortIndex = this.indexRegistry.findBestIndex(sortField, 'gte');
        if (sortIndex?.type === 'navigable') {
          indexedSort = true;
        }
      }
    }

    return {
      ...basePlan,
      indexedSort,
      sort:
        sortField && sortDirection
          ? { field: sortField, direction: sortDirection }
          : undefined,
      limit: options.limit,
      offset: options.offset,
    };
  }

  /**
   * Optimize a single query node.
   */
  private optimizeNode(query: Query): PlanStep {
    if (isLogicalQuery(query)) {
      return this.optimizeLogical(query);
    } else if (isSimpleQuery(query)) {
      return this.optimizeSimple(query);
    } else {
      // Unknown query type - fall back to full scan
      return { type: 'full-scan', predicate: query };
    }
  }

  /**
   * Optimize a simple (attribute-based) query.
   */
  private optimizeSimple(query: SimpleQueryNode): PlanStep {
    // Map query type to index query type
    const indexQueryType = this.mapQueryType(query.type);

    // Find best index for this attribute and query type
    const index = this.indexRegistry.findBestIndex(query.attribute, indexQueryType);

    if (index) {
      // Use index scan
      const indexQuery = this.buildIndexQuery(query);
      return { type: 'index-scan', index, query: indexQuery };
    }

    // No suitable index - fall back to full scan
    return { type: 'full-scan', predicate: query };
  }

  /**
   * Optimize a logical (AND/OR/NOT) query.
   */
  private optimizeLogical(query: LogicalQueryNode): PlanStep {
    switch (query.type) {
      case 'and':
        return this.optimizeAnd(query);
      case 'or':
        return this.optimizeOr(query);
      case 'not':
        return this.optimizeNot(query);
      default:
        throw new Error(`Unknown logical query type: ${query.type}`);
    }
  }

  /**
   * Optimize AND query.
   * Strategy: Find child with lowest cost, use as base, filter with rest.
   *
   * CQEngine "smallest first" strategy:
   * 1. Sort children by merge cost
   * 2. Use intersection if multiple indexes available
   * 3. Apply remaining predicates as filters
   */
  private optimizeAnd(query: LogicalQueryNode): PlanStep {
    if (!query.children || query.children.length === 0) {
      throw new Error('AND query must have children');
    }

    // Single child - just optimize it directly
    if (query.children.length === 1) {
      return this.optimizeNode(query.children[0]);
    }

    // Optimize all children
    const childSteps = query.children.map((child) => this.optimizeNode(child));

    // Sort by estimated cost (ascending)
    const sortedWithIndex = childSteps
      .map((step, index) => ({ step, originalIndex: index }))
      .sort((a, b) => this.estimateCost(a.step) - this.estimateCost(b.step));

    const sortedSteps = sortedWithIndex.map((s) => s.step);

    // Separate indexed steps from full scan steps
    const indexedSteps = sortedSteps.filter((s) => s.type === 'index-scan');
    const fullScanSteps = sortedSteps.filter((s) => s.type === 'full-scan');

    // No indexes available - fall back to single full scan
    if (indexedSteps.length === 0) {
      return { type: 'full-scan', predicate: query };
    }

    // One index available - use it as base, filter with remaining predicates
    if (indexedSteps.length === 1) {
      const [indexStep] = indexedSteps;

      if (fullScanSteps.length === 0) {
        return indexStep;
      }

      // Build filter predicate from remaining conditions
      const remainingPredicates = fullScanSteps.map((s) => {
        if (s.type === 'full-scan') {
          return s.predicate;
        }
        throw new Error('Unexpected step type in remaining predicates');
      });

      const filterPredicate: Query =
        remainingPredicates.length === 1
          ? remainingPredicates[0]
          : { type: 'and', children: remainingPredicates };

      return { type: 'filter', source: indexStep, predicate: filterPredicate };
    }

    // Multiple indexes available - use intersection
    // CQEngine strategy: iterate smallest, check membership in others
    return { type: 'intersection', steps: indexedSteps };
  }

  /**
   * Optimize OR query.
   * Strategy: Union of all child results with deduplication.
   */
  private optimizeOr(query: LogicalQueryNode): PlanStep {
    if (!query.children || query.children.length === 0) {
      throw new Error('OR query must have children');
    }

    // Single child - just optimize it directly
    if (query.children.length === 1) {
      return this.optimizeNode(query.children[0]);
    }

    const childSteps = query.children.map((child) => this.optimizeNode(child));

    // If all children are full scans, do a single full scan
    if (childSteps.every((s) => s.type === 'full-scan')) {
      return { type: 'full-scan', predicate: query };
    }

    // Create union of all results
    return { type: 'union', steps: childSteps };
  }

  /**
   * Optimize NOT query.
   * Strategy: Get all keys, subtract matching keys.
   */
  private optimizeNot(query: LogicalQueryNode): PlanStep {
    if (!query.child) {
      throw new Error('NOT query must have a child');
    }

    const childStep = this.optimizeNode(query.child);

    return {
      type: 'not',
      source: childStep,
      allKeys: () => new Set(), // Will be provided by executor at runtime
    };
  }

  /**
   * Map query type to index query type.
   * Some query types have different names in indexes.
   */
  private mapQueryType(type: string): string {
    const mapping: Record<string, string> = {
      eq: 'equal',
      neq: 'equal', // Will negate in execution
      gt: 'gt',
      gte: 'gte',
      lt: 'lt',
      lte: 'lte',
      in: 'in',
      has: 'has',
      like: 'like',
      regex: 'regex',
      between: 'between',
    };
    return mapping[type] ?? type;
  }

  /**
   * Build an IndexQuery from a SimpleQueryNode.
   */
  private buildIndexQuery(query: SimpleQueryNode): IndexQuery<unknown> {
    switch (query.type) {
      case 'eq':
      case 'neq':
        return { type: 'equal', value: query.value };
      case 'gt':
        return { type: 'gt', value: query.value };
      case 'gte':
        return { type: 'gte', value: query.value };
      case 'lt':
        return { type: 'lt', value: query.value };
      case 'lte':
        return { type: 'lte', value: query.value };
      case 'in':
        return { type: 'in', values: query.values };
      case 'has':
        return { type: 'has' };
      default:
        throw new Error(`Cannot build index query for type: ${query.type}`);
    }
  }

  /**
   * Estimate the execution cost of a plan step.
   */
  private estimateCost(step: PlanStep): number {
    switch (step.type) {
      case 'index-scan':
        return step.index.getRetrievalCost();

      case 'full-scan':
        return Number.MAX_SAFE_INTEGER;

      case 'intersection':
        // Cost is minimum of all (we only iterate smallest)
        return Math.min(...step.steps.map((s) => this.estimateCost(s)));

      case 'union':
        // Cost is sum of all
        return step.steps.reduce((sum, s) => {
          const cost = this.estimateCost(s);
          // Avoid overflow
          if (cost === Number.MAX_SAFE_INTEGER) {
            return Number.MAX_SAFE_INTEGER;
          }
          return Math.min(sum + cost, Number.MAX_SAFE_INTEGER);
        }, 0);

      case 'filter':
        // Filter adds overhead to source cost
        return this.estimateCost(step.source) + 10;

      case 'not':
        // NOT is expensive (needs all keys)
        return this.estimateCost(step.source) + 100;

      default:
        return Number.MAX_SAFE_INTEGER;
    }
  }

  /**
   * Check if a plan step uses any indexes.
   */
  private usesIndexes(step: PlanStep): boolean {
    switch (step.type) {
      case 'index-scan':
        return true;

      case 'full-scan':
        return false;

      case 'intersection':
      case 'union':
        return step.steps.some((s) => this.usesIndexes(s));

      case 'filter':
        return this.usesIndexes(step.source);

      case 'not':
        return this.usesIndexes(step.source);

      default:
        return false;
    }
  }
}
