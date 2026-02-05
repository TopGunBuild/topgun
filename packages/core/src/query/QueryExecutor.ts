/**
 * QueryExecutor Implementation
 *
 * Executes query plans produced by QueryOptimizer.
 * Supports hybrid queries with FTS and traditional predicates,
 * using Reciprocal Rank Fusion (RRF) for result merging.
 *
 * @module query/QueryExecutor
 */

import { QueryOptimizer } from './QueryOptimizer';
import type {
  Query,
  QueryPlan,
  PlanStep,
  IndexScanStep,
  FullScanStep,
  IntersectionStep,
  UnionStep,
  FilterStep,
  NotStep,
  FTSScanStep,
  FusionStep,
  FusionStrategy,
  SimpleQueryNode,
} from './QueryTypes';
import { isSimpleQuery, isLogicalQuery, isFTSQuery } from './QueryTypes';
import { ReciprocalRankFusion, type RankedResult, type MergedResult } from '../search/ReciprocalRankFusion';
import type { FullTextIndex } from '../fts';
import { SetResultSet } from './resultset/SetResultSet';
import type { ResultSet } from './resultset/ResultSet';
import { QueryCursor, type QueryCursorData } from './QueryCursor';
import { compareValues } from '../utils/compare';

/**
 * Result of executing a query step.
 * Contains keys with optional scores for ranked results.
 */
export interface StepResult<K = string> {
  /** Matching keys */
  keys: Set<K>;
  /** Optional scores for ranked results (key -> score) */
  scores?: Map<K, number>;
  /** Optional matched terms for FTS results (key -> terms) */
  matchedTerms?: Map<K, string[]>;
  /** Source type of this result */
  source: 'exact' | 'range' | 'fulltext' | 'standing';
}

/**
 * Query result with optional score and matched terms.
 */
export interface QueryResult<K, V> {
  /** Record key */
  key: K;
  /** Record value */
  value: V;
  /** BM25 or RRF score (for ranked results) */
  score?: number;
  /** Matched search terms (for FTS results) */
  matchedTerms?: string[];
}

/**
 * Order by specification.
 */
export interface OrderBy {
  /** Field to sort by (use '_score' for relevance sorting) */
  field: string;
  /** Sort direction */
  direction: 'asc' | 'desc';
}

/**
 * Options for query execution.
 */
export interface ExecuteOptions {
  /** Order by specifications */
  orderBy?: OrderBy[];
  /** Maximum results to return */
  limit?: number;
  /** Cursor for pagination (replaces offset) */
  cursor?: string;
  /** Query predicate (for cursor validation) */
  predicate?: Query;
}

/**
 * Cursor status for debugging.
 */
export type CursorStatus = 'valid' | 'expired' | 'invalid' | 'none';

/**
 * Extended query result with cursor info.
 */
export interface QueryResultWithCursor<K, V> {
  /** Query results */
  results: QueryResult<K, V>[];
  /** Cursor for next page (undefined if no more results) */
  nextCursor?: string;
  /** Whether more results are available */
  hasMore: boolean;
  /** Debug info: status of input cursor processing */
  cursorStatus: CursorStatus;
}

/**
 * Query executor that runs query plans.
 *
 * @example
 * ```typescript
 * const optimizer = new QueryOptimizer({ indexRegistry });
 * const executor = new QueryExecutor(optimizer);
 *
 * const query: Query = {
 *   type: 'and',
 *   children: [
 *     { type: 'eq', attribute: 'status', value: 'active' },
 *     { type: 'match', attribute: 'body', query: 'machine learning' },
 *   ],
 * };
 *
 * const results = await executor.execute(query, data);
 * // Returns results sorted by relevance
 * ```
 */
export class QueryExecutor<K extends string, V> {
  private readonly optimizer: QueryOptimizer<K, V>;
  private readonly rrf: ReciprocalRankFusion;

  constructor(
    optimizer: QueryOptimizer<K, V>,
    rrfConfig?: { k?: number }
  ) {
    this.optimizer = optimizer;
    this.rrf = new ReciprocalRankFusion(rrfConfig);
  }

  /**
   * Execute a query and return results.
   *
   * @param query - Query to execute
   * @param data - Data map to query
   * @param options - Execution options (orderBy, limit, cursor)
   * @returns Query results
   */
  execute(
    query: Query,
    data: Map<K, V>,
    options?: ExecuteOptions
  ): QueryResult<K, V>[] {
    const result = this.executeWithCursor(query, data, options);
    return result.results;
  }

  /**
   * Execute a query and return results with cursor information.
   * Use this method for paginated queries.
   *
   * @param query - Query to execute
   * @param data - Data map to query
   * @param options - Execution options (orderBy, limit, cursor)
   * @returns Query results with cursor info
   */
  executeWithCursor(
    query: Query,
    data: Map<K, V>,
    options?: ExecuteOptions
  ): QueryResultWithCursor<K, V> {
    // Get execution plan from optimizer
    const plan = this.optimizer.optimize(query);

    // Execute the plan
    const stepResult = this.executeStep(plan.root, data);

    // Convert to QueryResult array
    let results = this.stepResultToQueryResults(stepResult, data);

    // Determine sort configuration
    let sortField = '_score';
    let sortDirection: 'asc' | 'desc' = 'desc';

    if (options?.orderBy && options.orderBy.length > 0) {
      sortField = options.orderBy[0].field;
      sortDirection = options.orderBy[0].direction;
      results = this.applyOrdering(results, options.orderBy, data);
    } else if (stepResult.scores && stepResult.scores.size > 0) {
      // Default: sort by score descending if we have scores
      results = this.applyOrdering(results, [{ field: '_score', direction: 'desc' }], data);
    }

    // Create sort config for cursor
    const sort: Record<string, 'asc' | 'desc'> = { [sortField]: sortDirection };

    // Apply cursor filtering and track status
    let cursorStatus: CursorStatus = 'none';
    if (options?.cursor) {
      const cursorData = QueryCursor.decode(options.cursor);
      if (!cursorData) {
        cursorStatus = 'invalid';
      } else if (!QueryCursor.isValid(cursorData, options.predicate, sort)) {
        // Check if it's specifically expired vs hash mismatch
        const maxAge = 10 * 60 * 1000; // DEFAULT_QUERY_CURSOR_MAX_AGE_MS
        if (Date.now() - cursorData.timestamp > maxAge) {
          cursorStatus = 'expired';
        } else {
          cursorStatus = 'invalid';
        }
      } else {
        cursorStatus = 'valid';
        results = results.filter((result) => {
          const sortValue = this.extractSortValue(result, sortField);
          return QueryCursor.isAfterCursor(
            { key: result.key, sortValue },
            cursorData
          );
        });
      }
    }

    // Determine if there are more results
    const hasLimit = options?.limit !== undefined && options.limit > 0;
    const totalBeforeLimit = results.length;

    // Apply limit
    if (hasLimit) {
      results = results.slice(0, options.limit);
    }

    // Generate next cursor if there are more results
    let nextCursor: string | undefined;
    const hasMore = hasLimit && totalBeforeLimit > options.limit!;

    if (hasMore && results.length > 0) {
      const lastResult = results[results.length - 1];
      const sortValue = this.extractSortValue(lastResult, sortField);
      nextCursor = QueryCursor.fromLastResult(
        { key: lastResult.key, sortValue },
        sort,
        options?.predicate
      );
    }

    return {
      results,
      nextCursor,
      hasMore,
      cursorStatus,
    };
  }

  /**
   * Extract sort value from a query result.
   */
  private extractSortValue(result: QueryResult<K, V>, sortField: string): unknown {
    if (sortField === '_score') {
      return result.score ?? 0;
    }
    return (result.value as Record<string, unknown>)[sortField];
  }

  /**
   * Execute a plan step.
   */
  private executeStep(step: PlanStep, data: Map<K, V>): StepResult<K> {
    switch (step.type) {
      case 'point-lookup':
        return this.executePointLookup(step, data);

      case 'multi-point-lookup':
        return this.executeMultiPointLookup(step, data);

      case 'index-scan':
        return this.executeIndexScan(step, data);

      case 'full-scan':
        return this.executeFullScan(step, data);

      case 'intersection':
        return this.executeIntersection(step, data);

      case 'union':
        return this.executeUnion(step, data);

      case 'filter':
        return this.executeFilter(step, data);

      case 'not':
        return this.executeNot(step, data);

      case 'fts-scan':
        return this.executeFTSStep(step, data);

      case 'fusion':
        return this.executeFusion(step, data);

      default:
        throw new Error(`Unknown step type: ${(step as PlanStep).type}`);
    }
  }

  /**
   * Execute a point lookup step - O(1) direct key access.
   */
  private executePointLookup(step: import('./QueryTypes').PointLookupStep, data: Map<K, V>): StepResult<K> {
    const key = step.key as K;
    const keys = new Set<K>();

    if (data.has(key)) {
      keys.add(key);
    }

    return {
      keys,
      source: 'exact',
    };
  }

  /**
   * Execute a multi-point lookup step - O(k) batch key access.
   */
  private executeMultiPointLookup(step: import('./QueryTypes').MultiPointLookupStep, data: Map<K, V>): StepResult<K> {
    const keys = new Set<K>();

    for (const key of step.keys) {
      const k = key as K;
      if (data.has(k)) {
        keys.add(k);
      }
    }

    return {
      keys,
      source: 'exact',
    };
  }

  /**
   * Execute an index scan step.
   */
  private executeIndexScan(step: IndexScanStep, _data: Map<K, V>): StepResult<K> {
    const resultSet = step.index.retrieve(step.query) as ResultSet<K>;
    const keys = new Set(resultSet.toArray());

    return {
      keys,
      source: 'exact',
    };
  }

  /**
   * Execute a full scan step.
   */
  private executeFullScan(step: FullScanStep, data: Map<K, V>): StepResult<K> {
    const keys = new Set<K>();

    for (const [key, value] of data) {
      if (this.evaluatePredicate(step.predicate, value)) {
        keys.add(key);
      }
    }

    return {
      keys,
      source: 'exact',
    };
  }

  /**
   * Execute an FTS scan step using FullTextIndex.search().
   */
  private executeFTSStep(step: FTSScanStep, _data: Map<K, V>): StepResult<K> {
    const ftsIndex = this.optimizer.getFullTextIndex(step.field);

    if (!ftsIndex) {
      // No FTS index - return empty result
      return {
        keys: new Set(),
        source: 'fulltext',
      };
    }

    // Execute search based on FTS type
    const searchResults = ftsIndex.search(step.query, {
      minScore: step.options?.minScore,
      boost: step.options?.boost ? { [step.field]: step.options.boost } : undefined,
    });

    // Convert search results to StepResult
    const keys = new Set<K>();
    const scores = new Map<K, number>();
    const matchedTerms = new Map<K, string[]>();

    for (const result of searchResults) {
      const key = result.docId as K;
      keys.add(key);
      scores.set(key, result.score);
      if (result.matchedTerms) {
        matchedTerms.set(key, result.matchedTerms);
      }
    }

    return {
      keys,
      scores,
      matchedTerms,
      source: 'fulltext',
    };
  }

  /**
   * Execute an intersection step.
   */
  private executeIntersection(step: IntersectionStep, data: Map<K, V>): StepResult<K> {
    if (step.steps.length === 0) {
      return { keys: new Set(), source: 'exact' };
    }

    // Execute all child steps
    const childResults = step.steps.map((s) => this.executeStep(s, data));

    // Find smallest result set for efficient intersection
    childResults.sort((a, b) => a.keys.size - b.keys.size);

    // Start with smallest set
    const result = new Set<K>(childResults[0].keys);

    // Intersect with remaining sets
    for (let i = 1; i < childResults.length; i++) {
      const otherKeys = childResults[i].keys;
      for (const key of result) {
        if (!otherKeys.has(key)) {
          result.delete(key);
        }
      }
    }

    return {
      keys: result,
      source: 'exact',
    };
  }

  /**
   * Execute a union step.
   */
  private executeUnion(step: UnionStep, data: Map<K, V>): StepResult<K> {
    const result = new Set<K>();

    for (const childStep of step.steps) {
      const childResult = this.executeStep(childStep, data);
      for (const key of childResult.keys) {
        result.add(key);
      }
    }

    return {
      keys: result,
      source: 'exact',
    };
  }

  /**
   * Execute a filter step.
   */
  private executeFilter(step: FilterStep, data: Map<K, V>): StepResult<K> {
    const sourceResult = this.executeStep(step.source, data);
    const filtered = new Set<K>();
    const scores = sourceResult.scores ? new Map<K, number>() : undefined;
    const matchedTerms = sourceResult.matchedTerms ? new Map<K, string[]>() : undefined;

    for (const key of sourceResult.keys) {
      const value = data.get(key);
      if (value && this.evaluatePredicate(step.predicate, value)) {
        filtered.add(key);
        if (scores && sourceResult.scores?.has(key)) {
          scores.set(key, sourceResult.scores.get(key)!);
        }
        if (matchedTerms && sourceResult.matchedTerms?.has(key)) {
          matchedTerms.set(key, sourceResult.matchedTerms.get(key)!);
        }
      }
    }

    return {
      keys: filtered,
      scores,
      matchedTerms,
      source: sourceResult.source,
    };
  }

  /**
   * Execute a NOT step.
   */
  private executeNot(step: NotStep, data: Map<K, V>): StepResult<K> {
    const sourceResult = this.executeStep(step.source, data);
    const allKeys = new Set<K>(data.keys());
    const result = new Set<K>();

    for (const key of allKeys) {
      if (!sourceResult.keys.has(key)) {
        result.add(key);
      }
    }

    return {
      keys: result,
      source: 'exact',
    };
  }

  /**
   * Execute a fusion step.
   * Combines results from multiple steps using the specified strategy.
   */
  private executeFusion(step: FusionStep, data: Map<K, V>): StepResult<K> {
    const childResults = step.steps.map((s) => this.executeStep(s, data));

    return this.fuseResults(childResults, step.strategy);
  }

  /**
   * Fuse results using the specified strategy.
   *
   * @param stepResults - Results from multiple steps
   * @param strategy - Fusion strategy
   * @returns Fused result
   */
  fuseResults(
    stepResults: StepResult<K>[],
    strategy: FusionStrategy
  ): StepResult<K> {
    switch (strategy) {
      case 'intersection':
        return this.intersectResults(stepResults);

      case 'rrf':
        return this.rrfFusion(stepResults);

      case 'score-filter':
        return this.scoreSumFusion(stepResults);

      default:
        throw new Error(`Unknown fusion strategy: ${strategy}`);
    }
  }

  /**
   * Intersect multiple result sets (binary fusion).
   */
  private intersectResults(stepResults: StepResult<K>[]): StepResult<K> {
    if (stepResults.length === 0) {
      return { keys: new Set(), source: 'exact' };
    }

    // Sort by size for efficient intersection
    stepResults.sort((a, b) => a.keys.size - b.keys.size);

    const result = new Set<K>(stepResults[0].keys);

    for (let i = 1; i < stepResults.length; i++) {
      const otherKeys = stepResults[i].keys;
      for (const key of result) {
        if (!otherKeys.has(key)) {
          result.delete(key);
        }
      }
    }

    return {
      keys: result,
      source: 'exact',
    };
  }

  /**
   * RRF fusion for mixed binary and scored results.
   */
  private rrfFusion(stepResults: StepResult<K>[]): StepResult<K> {
    // Convert step results to ranked result sets for RRF
    const rankedSets: RankedResult[][] = [];

    for (const stepResult of stepResults) {
      const rankedResults: RankedResult[] = [];

      if (stepResult.scores && stepResult.scores.size > 0) {
        // Scored results - sort by score
        const sorted = Array.from(stepResult.scores.entries())
          .sort((a, b) => b[1] - a[1]);

        for (const [key, score] of sorted) {
          rankedResults.push({
            docId: key,
            score,
            source: stepResult.source,
          });
        }
      } else {
        // Binary results - assign equal scores
        for (const key of stepResult.keys) {
          rankedResults.push({
            docId: key,
            score: 1.0,
            source: stepResult.source,
          });
        }
      }

      if (rankedResults.length > 0) {
        rankedSets.push(rankedResults);
      }
    }

    // Use RRF to merge
    const merged = this.rrf.merge(rankedSets);

    // Convert back to StepResult
    const keys = new Set<K>();
    const scores = new Map<K, number>();
    const matchedTerms = new Map<K, string[]>();

    for (const result of merged) {
      const key = result.docId as K;
      keys.add(key);
      scores.set(key, result.score);
    }

    // Collect matched terms from all step results
    for (const stepResult of stepResults) {
      if (stepResult.matchedTerms) {
        for (const [key, terms] of stepResult.matchedTerms) {
          if (keys.has(key)) {
            const existing = matchedTerms.get(key) || [];
            const combined = new Set([...existing, ...terms]);
            matchedTerms.set(key, Array.from(combined));
          }
        }
      }
    }

    return {
      keys,
      scores,
      matchedTerms: matchedTerms.size > 0 ? matchedTerms : undefined,
      source: 'fulltext',
    };
  }

  /**
   * Score-sum fusion for all-scored results.
   */
  private scoreSumFusion(stepResults: StepResult<K>[]): StepResult<K> {
    const scoreAccumulator = new Map<K, number>();
    const matchedTermsAccumulator = new Map<K, Set<string>>();

    for (const stepResult of stepResults) {
      if (stepResult.scores) {
        for (const [key, score] of stepResult.scores) {
          const existing = scoreAccumulator.get(key) || 0;
          scoreAccumulator.set(key, existing + score);
        }
      } else {
        // Binary result - add 1.0 for each matching key
        for (const key of stepResult.keys) {
          const existing = scoreAccumulator.get(key) || 0;
          scoreAccumulator.set(key, existing + 1.0);
        }
      }

      // Collect matched terms
      if (stepResult.matchedTerms) {
        for (const [key, terms] of stepResult.matchedTerms) {
          const existing = matchedTermsAccumulator.get(key) || new Set();
          for (const term of terms) {
            existing.add(term);
          }
          matchedTermsAccumulator.set(key, existing);
        }
      }
    }

    const keys = new Set<K>(scoreAccumulator.keys());
    const matchedTerms = new Map<K, string[]>();

    for (const [key, terms] of matchedTermsAccumulator) {
      matchedTerms.set(key, Array.from(terms));
    }

    return {
      keys,
      scores: scoreAccumulator,
      matchedTerms: matchedTerms.size > 0 ? matchedTerms : undefined,
      source: 'fulltext',
    };
  }

  /**
   * Apply ordering to results.
   */
  applyOrdering(
    results: QueryResult<K, V>[],
    orderBy: OrderBy[],
    data: Map<K, V>
  ): QueryResult<K, V>[] {
    if (orderBy.length === 0) {
      return results;
    }

    return [...results].sort((a, b) => {
      for (const order of orderBy) {
        let comparison = 0;

        if (order.field === '_score') {
          // Sort by score
          const scoreA = a.score ?? 0;
          const scoreB = b.score ?? 0;
          comparison = scoreA - scoreB;
        } else {
          // Sort by field value
          const valueA = (a.value as Record<string, unknown>)[order.field];
          const valueB = (b.value as Record<string, unknown>)[order.field];
          comparison = this.compareValues(valueA, valueB);
        }

        if (comparison !== 0) {
          return order.direction === 'desc' ? -comparison : comparison;
        }
      }
      return 0;
    });
  }

  /**
   * Compare two values for sorting.
   * Delegates to shared compareValues utility.
   */
  private compareValues(a: unknown, b: unknown): number {
    return compareValues(a, b);
  }

  /**
   * Convert StepResult to QueryResult array.
   */
  private stepResultToQueryResults(
    stepResult: StepResult<K>,
    data: Map<K, V>
  ): QueryResult<K, V>[] {
    const results: QueryResult<K, V>[] = [];

    for (const key of stepResult.keys) {
      const value = data.get(key);
      if (value) {
        const result: QueryResult<K, V> = {
          key,
          value,
        };

        if (stepResult.scores?.has(key)) {
          result.score = stepResult.scores.get(key);
        }

        if (stepResult.matchedTerms?.has(key)) {
          result.matchedTerms = stepResult.matchedTerms.get(key);
        }

        results.push(result);
      }
    }

    return results;
  }

  /**
   * Evaluate a predicate against a record.
   */
  private evaluatePredicate(predicate: Query, record: V): boolean {
    const rec = record as Record<string, unknown>;

    if (isSimpleQuery(predicate)) {
      return this.evaluateSimplePredicate(predicate, rec);
    }

    if (isLogicalQuery(predicate)) {
      switch (predicate.type) {
        case 'and':
          return predicate.children?.every((c) => this.evaluatePredicate(c, record)) ?? true;
        case 'or':
          return predicate.children?.some((c) => this.evaluatePredicate(c, record)) ?? false;
        case 'not':
          return predicate.child ? !this.evaluatePredicate(predicate.child, record) : true;
      }
    }

    if (isFTSQuery(predicate)) {
      // FTS predicates are handled by FTS index, not evaluated here
      // For fallback full-scan, we do simple substring match
      const field = predicate.attribute;
      const fieldValue = rec[field];

      if (typeof fieldValue !== 'string') {
        return false;
      }

      const searchText = predicate.type === 'matchPrefix'
        ? predicate.prefix
        : predicate.query;

      // Simple case-insensitive substring match as fallback
      return fieldValue.toLowerCase().includes(searchText.toLowerCase());
    }

    return false;
  }

  /**
   * Evaluate a simple predicate.
   */
  private evaluateSimplePredicate(predicate: SimpleQueryNode, record: Record<string, unknown>): boolean {
    const fieldValue = record[predicate.attribute];

    switch (predicate.type) {
      case 'eq':
        return fieldValue === predicate.value;

      case 'neq':
        return fieldValue !== predicate.value;

      case 'gt':
        return (fieldValue as number) > (predicate.value as number);

      case 'gte':
        return (fieldValue as number) >= (predicate.value as number);

      case 'lt':
        return (fieldValue as number) < (predicate.value as number);

      case 'lte':
        return (fieldValue as number) <= (predicate.value as number);

      case 'in':
        return predicate.values?.includes(fieldValue) ?? false;

      case 'between': {
        const val = fieldValue as number;
        const from = predicate.from as number;
        const to = predicate.to as number;
        const fromInclusive = predicate.fromInclusive ?? true;
        const toInclusive = predicate.toInclusive ?? false;

        const passesFrom = fromInclusive ? val >= from : val > from;
        const passesTo = toInclusive ? val <= to : val < to;
        return passesFrom && passesTo;
      }

      case 'like': {
        if (typeof fieldValue !== 'string' || typeof predicate.value !== 'string') {
          return false;
        }
        // Simple LIKE implementation: % = any, _ = single char
        const pattern = (predicate.value as string)
          .replace(/%/g, '.*')
          .replace(/_/g, '.');
        const regex = new RegExp(`^${pattern}$`, 'i');
        return regex.test(fieldValue);
      }

      case 'regex': {
        if (typeof fieldValue !== 'string') {
          return false;
        }
        const regex = new RegExp(predicate.value as string);
        return regex.test(fieldValue);
      }

      case 'has':
        return fieldValue !== undefined && fieldValue !== null;

      case 'contains': {
        if (Array.isArray(fieldValue)) {
          return fieldValue.includes(predicate.value);
        }
        return false;
      }

      case 'containsAll': {
        if (!Array.isArray(fieldValue) || !predicate.values) {
          return false;
        }
        return predicate.values.every((v) => fieldValue.includes(v));
      }

      case 'containsAny': {
        if (!Array.isArray(fieldValue) || !predicate.values) {
          return false;
        }
        return predicate.values.some((v) => fieldValue.includes(v));
      }

      default:
        return false;
    }
  }
}
