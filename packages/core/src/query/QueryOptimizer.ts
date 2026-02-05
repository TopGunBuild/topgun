/**
 * QueryOptimizer Implementation
 *
 * Cost-based query optimizer for the Query Engine.
 * Selects optimal index and execution strategy for queries.
 *
 * Algorithm based on CQEngine CollectionQueryEngine:
 * - StandingQueryIndex: Check first (lowest cost = 10)
 * - AND queries: "smallest first" strategy - sort by merge cost, iterate smallest
 * - OR queries: Union all results with deduplication
 * - NOT queries: Get all keys, subtract matching keys
 *
 * @module query/QueryOptimizer
 */

import { IndexRegistry } from './IndexRegistry';
import { StandingQueryRegistry } from './StandingQueryRegistry';
import type {
  Query,
  SimpleQueryNode,
  LogicalQueryNode,
  QueryPlan,
  PlanStep,
  QueryOptions,
  FTSQueryNode,
  FTSScanStep,
  FusionStep,
  FusionStrategy,
  MatchQueryNode,
  MatchPhraseQueryNode,
  MatchPrefixQueryNode,
  QueryContext,
  DistributedCost,
} from './QueryTypes';
import { isLogicalQuery, isSimpleQuery, isFTSQuery, calculateTotalCost } from './QueryTypes';
import type { FullTextIndex } from '../fts';
import type { IndexQuery } from './indexes/types';
import type { CompoundIndex } from './indexes/CompoundIndex';

/**
 * Options for creating a QueryOptimizer.
 */
export interface QueryOptimizerOptions<K, V> {
  /** Index registry for attribute-based indexes */
  indexRegistry: IndexRegistry<K, V>;
  /** Standing query registry for pre-computed queries (optional) */
  standingQueryRegistry?: StandingQueryRegistry<K, V>;
  /** Full-text index registry for FTS queries */
  fullTextIndexes?: Map<string, FullTextIndex>;
}

/**
 * Classified predicates by type for hybrid query planning.
 */
export interface ClassifiedPredicates {
  /** Exact match predicates (eq, neq, in) */
  exactPredicates: Query[];
  /** Range predicates (gt, gte, lt, lte, between) */
  rangePredicates: Query[];
  /** Full-text search predicates (match, matchPhrase, matchPrefix) */
  ftsPredicates: FTSQueryNode[];
  /** Other predicates (like, regex, contains, etc.) */
  otherPredicates: Query[];
}

/**
 * Cost-based query optimizer.
 * Selects optimal index and execution strategy for queries.
 *
 * K = record key type, V = record value type
 */
export class QueryOptimizer<K, V> {
  private readonly indexRegistry: IndexRegistry<K, V>;
  private readonly standingQueryRegistry?: StandingQueryRegistry<K, V>;
  private readonly fullTextIndexes: Map<string, FullTextIndex>;

  /**
   * Create a QueryOptimizer.
   *
   * @param options - QueryOptimizer options
   */
  constructor(options: QueryOptimizerOptions<K, V>) {
    this.indexRegistry = options.indexRegistry;
    this.standingQueryRegistry = options.standingQueryRegistry;
    this.fullTextIndexes = options.fullTextIndexes ?? new Map();
  }

  /**
   * Register a full-text index for a field.
   *
   * @param field - Field name
   * @param index - FullTextIndex instance
   */
  registerFullTextIndex(field: string, index: FullTextIndex): void {
    this.fullTextIndexes.set(field, index);
  }

  /**
   * Unregister a full-text index.
   *
   * @param field - Field name
   */
  unregisterFullTextIndex(field: string): void {
    this.fullTextIndexes.delete(field);
  }

  /**
   * Get registered full-text index for a field.
   *
   * @param field - Field name
   * @returns FullTextIndex or undefined
   */
  getFullTextIndex(field: string): FullTextIndex | undefined {
    return this.fullTextIndexes.get(field);
  }

  /**
   * Check if a full-text index exists for a field.
   *
   * @param field - Field name
   * @returns True if FTS index exists
   */
  hasFullTextIndex(field: string): boolean {
    return this.fullTextIndexes.has(field);
  }

  /**
   * Optimize a query and return an execution plan.
   *
   * Optimization order (by cost):
   * 1. Point lookup (cost: 1) - direct primary key access
   * 2. StandingQueryIndex (cost: 10) - pre-computed results
   * 3. Other indexes via optimizeNode
   *
   * @param query - Query to optimize
   * @returns Query execution plan
   */
  optimize(query: Query): QueryPlan {
    // Check for point lookup first (absolute lowest cost)
    const pointLookupStep = this.tryPointLookup(query);
    if (pointLookupStep) {
      return {
        root: pointLookupStep,
        estimatedCost: this.estimateCost(pointLookupStep),
        usesIndexes: this.usesIndexes(pointLookupStep),
      };
    }

    // Check for standing query index second (low cost)
    if (this.standingQueryRegistry) {
      const standingIndex = this.standingQueryRegistry.getIndex(query);
      if (standingIndex) {
        return {
          root: {
            type: 'index-scan',
            index: standingIndex,
            query: { type: 'equal', value: null }, // Dummy query, index returns pre-computed results
          },
          estimatedCost: standingIndex.getRetrievalCost(),
          usesIndexes: true,
        };
      }
    }

    // Fall back to regular optimization
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
    if (!options.sort && options.limit === undefined && options.cursor === undefined) {
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
      cursor: options.cursor, // replaces offset
    };
  }

  /**
   * Try to optimize query as a point lookup.
   * Returns a point lookup step if query is an equality or IN query on primary key.
   *
   * @param query - Query to check
   * @returns Point lookup step or null
   */
  private tryPointLookup(query: Query): PlanStep | null {
    // Only simple queries can be point lookups
    if (!isSimpleQuery(query)) {
      return null;
    }

    // Check if attribute is a primary key field
    const primaryKeyFields = ['_key', 'key', 'id'];
    if (!primaryKeyFields.includes(query.attribute)) {
      return null;
    }

    // Handle 'eq' type - single point lookup
    if (query.type === 'eq') {
      return {
        type: 'point-lookup',
        key: query.value,
        cost: 1,
      };
    }

    // Handle 'in' type - multi-point lookup
    if (query.type === 'in' && query.values) {
      return {
        type: 'multi-point-lookup',
        keys: query.values,
        cost: query.values.length,
      };
    }

    return null;
  }

  /**
   * Optimize a single query node.
   */
  private optimizeNode(query: Query): PlanStep {
    if (isLogicalQuery(query)) {
      return this.optimizeLogical(query);
    } else if (isFTSQuery(query)) {
      return this.optimizeFTS(query);
    } else if (isSimpleQuery(query)) {
      return this.optimizeSimple(query);
    } else {
      // Unknown query type - fall back to full scan
      return { type: 'full-scan', predicate: query };
    }
  }

  /**
   * Optimize a full-text search query.
   */
  private optimizeFTS(query: FTSQueryNode): PlanStep {
    const field = query.attribute;

    // Check if we have a FTS index for this field
    if (!this.hasFullTextIndex(field)) {
      // No FTS index - fall back to full scan
      return { type: 'full-scan', predicate: query };
    }

    // Create FTS scan step
    return this.buildFTSScanStep(query);
  }

  /**
   * Build an FTS scan step from a query node.
   */
  private buildFTSScanStep(query: FTSQueryNode): FTSScanStep {
    const field = query.attribute;

    switch (query.type) {
      case 'match':
        return {
          type: 'fts-scan',
          field,
          query: query.query,
          ftsType: 'match',
          options: query.options,
          returnsScored: true,
          estimatedCost: this.estimateFTSCost(field),
        };

      case 'matchPhrase':
        return {
          type: 'fts-scan',
          field,
          query: query.query,
          ftsType: 'matchPhrase',
          options: query.slop !== undefined ? { fuzziness: query.slop } : undefined,
          returnsScored: true,
          estimatedCost: this.estimateFTSCost(field),
        };

      case 'matchPrefix':
        return {
          type: 'fts-scan',
          field,
          query: query.prefix,
          ftsType: 'matchPrefix',
          options: query.maxExpansions !== undefined ? { fuzziness: query.maxExpansions } : undefined,
          returnsScored: true,
          estimatedCost: this.estimateFTSCost(field),
        };

      default:
        throw new Error(`Unknown FTS query type: ${(query as FTSQueryNode).type}`);
    }
  }

  /**
   * Estimate cost of FTS query based on index size.
   */
  private estimateFTSCost(field: string): number {
    const index = this.fullTextIndexes.get(field);
    if (!index) {
      return Number.MAX_SAFE_INTEGER;
    }

    // FTS cost is based on document count
    // Roughly O(log N) for term lookup + O(M) for scoring M matching docs
    const docCount = index.getSize();

    // Base cost + log scale factor
    return 50 + Math.log2(docCount + 1) * 10;
  }

  /**
   * Classify predicates by type for hybrid query planning.
   *
   * @param predicates - Array of predicates to classify
   * @returns Classified predicates
   */
  classifyPredicates(predicates: Query[]): ClassifiedPredicates {
    const result: ClassifiedPredicates = {
      exactPredicates: [],
      rangePredicates: [],
      ftsPredicates: [],
      otherPredicates: [],
    };

    for (const pred of predicates) {
      if (isFTSQuery(pred)) {
        result.ftsPredicates.push(pred);
      } else if (isSimpleQuery(pred)) {
        switch (pred.type) {
          case 'eq':
          case 'neq':
          case 'in':
            result.exactPredicates.push(pred);
            break;
          case 'gt':
          case 'gte':
          case 'lt':
          case 'lte':
          case 'between':
            result.rangePredicates.push(pred);
            break;
          default:
            result.otherPredicates.push(pred);
        }
      } else if (isLogicalQuery(pred)) {
        // Logical predicates go to other
        result.otherPredicates.push(pred);
      } else {
        result.otherPredicates.push(pred);
      }
    }

    return result;
  }

  /**
   * Determine fusion strategy based on step types.
   *
   * Strategy selection:
   * - All binary (exact/range with no scores) → 'intersection'
   * - All scored (FTS) → 'score-filter' (filter by score, sort by score)
   * - Mixed (binary + scored) → 'rrf' (Reciprocal Rank Fusion)
   *
   * @param steps - Plan steps to fuse
   * @returns Fusion strategy
   */
  determineFusionStrategy(steps: PlanStep[]): FusionStrategy {
    const hasScored = steps.some((s) => this.stepReturnsScored(s));
    const hasBinary = steps.some((s) => !this.stepReturnsScored(s));

    if (hasScored && hasBinary) {
      // Mixed: use RRF to combine ranked and unranked results
      return 'rrf';
    } else if (hasScored) {
      // All scored: filter by score, combine scores
      return 'score-filter';
    } else {
      // All binary: simple intersection
      return 'intersection';
    }
  }

  /**
   * Check if a plan step returns scored results.
   */
  private stepReturnsScored(step: PlanStep): boolean {
    switch (step.type) {
      case 'fts-scan':
        return true;
      case 'fusion':
        return step.returnsScored;
      default:
        return false;
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
   * 1. Check for CompoundIndex covering all eq children
   * 2. Sort children by merge cost
   * 3. Use intersection if multiple indexes available
   * 4. Apply remaining predicates as filters
   */
  private optimizeAnd(query: LogicalQueryNode): PlanStep {
    if (!query.children || query.children.length === 0) {
      throw new Error('AND query must have children');
    }

    // Single child - just optimize it directly
    if (query.children.length === 1) {
      return this.optimizeNode(query.children[0]);
    }

    // Check if a CompoundIndex can handle this AND query
    const compoundStep = this.tryCompoundIndex(query.children);
    if (compoundStep) {
      return compoundStep;
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
   * Try to use a CompoundIndex for an AND query.
   *
   * Returns a compound index scan step if:
   * 1. All children are simple 'eq' queries
   * 2. A CompoundIndex exists covering all queried attributes
   *
   * @param children - Children of the AND query
   * @returns IndexScanStep using CompoundIndex, or null if not applicable
   */
  private tryCompoundIndex(children: Query[]): PlanStep | null {
    // Check if all children are simple 'eq' queries
    const eqQueries: SimpleQueryNode[] = [];
    const otherQueries: Query[] = [];

    for (const child of children) {
      if (isSimpleQuery(child) && child.type === 'eq') {
        eqQueries.push(child);
      } else {
        otherQueries.push(child);
      }
    }

    // Need at least 2 'eq' queries to use compound index
    if (eqQueries.length < 2) {
      return null;
    }

    // Extract attribute names from eq queries
    const attributeNames = eqQueries.map((q) => q.attribute);

    // Find a compound index covering these attributes
    const compoundIndex = this.indexRegistry.findCompoundIndex(attributeNames);
    if (!compoundIndex) {
      return null;
    }

    // Build values array in the order expected by the compound index
    const values = this.buildCompoundValues(compoundIndex, eqQueries);
    if (!values) {
      return null; // Attribute order mismatch
    }

    // Create compound index scan step
    const compoundStep: PlanStep = {
      type: 'index-scan',
      index: compoundIndex as unknown as import('./indexes/types').Index<unknown, unknown, unknown>,
      query: { type: 'compound', values },
    };

    // If there are other (non-eq) queries, apply them as filters
    if (otherQueries.length > 0) {
      const filterPredicate: Query =
        otherQueries.length === 1
          ? otherQueries[0]
          : { type: 'and', children: otherQueries };

      return { type: 'filter', source: compoundStep, predicate: filterPredicate };
    }

    return compoundStep;
  }

  /**
   * Build values array for compound index query in correct attribute order.
   *
   * @param compoundIndex - The compound index to use
   * @param eqQueries - Array of 'eq' queries
   * @returns Values array in compound index order, or null if mismatch
   */
  private buildCompoundValues(
    compoundIndex: CompoundIndex<K, V>,
    eqQueries: SimpleQueryNode[]
  ): unknown[] | null {
    const attributeNames = compoundIndex.attributes.map((a) => a.name);
    const values: unknown[] = [];

    // Build a map of attribute -> value from eq queries
    const queryMap = new Map<string, unknown>();
    for (const q of eqQueries) {
      queryMap.set(q.attribute, q.value);
    }

    // Build values array in compound index order
    for (const attrName of attributeNames) {
      if (!queryMap.has(attrName)) {
        return null; // Missing attribute value
      }
      values.push(queryMap.get(attrName));
    }

    return values;
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
      contains: 'contains',
      containsAll: 'containsAll',
      containsAny: 'containsAny',
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
      case 'between':
        return {
          type: 'between',
          from: query.from,
          to: query.to,
          fromInclusive: query.fromInclusive,
          toInclusive: query.toInclusive,
        };
      case 'contains':
        return { type: 'contains', value: query.value };
      case 'containsAll':
        return { type: 'containsAll', values: query.values };
      case 'containsAny':
        return { type: 'containsAny', values: query.values };
      default:
        throw new Error(`Cannot build index query for type: ${query.type}`);
    }
  }

  /**
   * Estimate the execution cost of a plan step.
   */
  private estimateCost(step: PlanStep): number {
    switch (step.type) {
      case 'point-lookup':
      case 'multi-point-lookup':
        return step.cost;

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

      // FTS step types
      case 'fts-scan':
        return step.estimatedCost;

      case 'fusion':
        // Fusion cost is sum of all child costs + fusion overhead
        return step.steps.reduce((sum, s) => {
          const cost = this.estimateCost(s);
          if (cost === Number.MAX_SAFE_INTEGER) {
            return Number.MAX_SAFE_INTEGER;
          }
          return Math.min(sum + cost, Number.MAX_SAFE_INTEGER);
        }, 0) + 20; // Fusion overhead

      default:
        return Number.MAX_SAFE_INTEGER;
    }
  }

  /**
   * Estimate distributed cost including network overhead.
   *
   * Network cost is assigned based on step type:
   * - full-scan: broadcast to all nodes (highest cost)
   * - index-scan: 0 if local partition, 5 if remote
   * - point-lookup: 0 if local key, 5 if remote
   * - intersection/union: aggregating results from multiple sources
   *
   * @param step - Plan step to estimate
   * @param context - Distributed query context (optional)
   * @returns Distributed cost breakdown
   */
  estimateDistributedCost(step: PlanStep, context?: QueryContext): DistributedCost {
    const baseCost = this.estimateCost(step);

    // If no context or single node, no network cost
    if (!context?.isDistributed || context.nodeCount <= 1) {
      return {
        rows: baseCost,
        cpu: baseCost,
        network: 0,
        io: 0,
      };
    }

    // Estimate network cost based on step type
    let networkCost = 0;

    switch (step.type) {
      case 'full-scan':
        // Full scan requires broadcasting query to all nodes
        networkCost = context.nodeCount * 10;
        break;

      case 'index-scan':
        // Index scan may be local or require network hop
        networkCost = 5; // Assume remote by default
        break;

      case 'point-lookup':
        // Point lookup: one network hop if remote
        networkCost = 5; // Assume remote by default
        break;

      case 'multi-point-lookup':
        // Multiple point lookups may hit multiple partitions
        networkCost = Math.min(step.keys.length, context.nodeCount) * 5;
        break;

      case 'intersection':
      case 'union':
        // Aggregating results from multiple sources
        networkCost = step.steps.length * 5;
        break;

      case 'filter':
        // Filter inherits source network cost
        return this.estimateDistributedCost(step.source, context);

      case 'not':
        // NOT needs all keys plus source
        networkCost = context.nodeCount * 5;
        break;

      case 'fts-scan':
        // FTS typically broadcasts to nodes with index shards
        networkCost = Math.ceil(context.nodeCount / 2) * 5;
        break;

      case 'fusion':
        // Sum of child step costs
        networkCost = step.steps.reduce(
          (sum, s) => sum + this.estimateDistributedCost(s, context).network,
          0
        );
        break;
    }

    return {
      rows: baseCost,
      cpu: baseCost,
      network: networkCost,
      io: context.usesStorage ? baseCost * 0.5 : 0,
    };
  }

  /**
   * Get total distributed cost for a plan step.
   * Convenience method combining estimateDistributedCost and calculateTotalCost.
   *
   * @param step - Plan step to estimate
   * @param context - Distributed query context (optional)
   * @returns Weighted total cost
   */
  getTotalDistributedCost(step: PlanStep, context?: QueryContext): number {
    const distributedCost = this.estimateDistributedCost(step, context);
    return calculateTotalCost(distributedCost);
  }

  /**
   * Check if a plan step uses any indexes.
   */
  private usesIndexes(step: PlanStep): boolean {
    switch (step.type) {
      case 'point-lookup':
      case 'multi-point-lookup':
        return true;

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

      // FTS step types
      case 'fts-scan':
        return true; // FTS uses FullTextIndex

      case 'fusion':
        return step.steps.some((s) => this.usesIndexes(s));

      default:
        return false;
    }
  }
}
