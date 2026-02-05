/**
 * IndexedLWWMap Implementation
 *
 * LWWMap with index support for O(1) to O(log N) queries.
 * Wraps LWWMap with indexing capabilities using the Wrapper Pattern.
 *
 * Features:
 * - Hash and Navigable indexes for efficient queries
 * - Live queries with StandingQueryIndex
 * - Automatic index updates on CRDT operations
 * - Cost-based query optimization
 * - Adaptive indexing with query pattern tracking
 *
 * @module IndexedLWWMap
 */

import { LWWMap, LWWRecord } from './LWWMap';
import { HLC } from './HLC';
import { IndexRegistry, IndexRegistryStats } from './query/IndexRegistry';
import { QueryOptimizer } from './query/QueryOptimizer';
import { StandingQueryRegistry } from './query/StandingQueryRegistry';
import {
  LiveQueryManager,
  LiveQueryCallback,
  LiveQueryEvent,
} from './query/LiveQueryManager';
import type { Index, IndexStats, IndexQuery } from './query/indexes/types';
import { HashIndex } from './query/indexes/HashIndex';
import { NavigableIndex } from './query/indexes/NavigableIndex';
import { FallbackIndex } from './query/indexes/FallbackIndex';
import { InvertedIndex } from './query/indexes/InvertedIndex';
import { TokenizationPipeline } from './query/tokenization';
import {
  LazyHashIndex,
  LazyNavigableIndex,
  LazyInvertedIndex,
} from './query/indexes/lazy';
import { Attribute, simpleAttribute } from './query/Attribute';
import type { Query, QueryPlan, PlanStep, SimpleQueryNode } from './query/QueryTypes';
import { isSimpleQuery } from './query/QueryTypes';
import type { ResultSet } from './query/resultset/ResultSet';
import { SetResultSet } from './query/resultset/SetResultSet';
import { IntersectionResultSet } from './query/resultset/IntersectionResultSet';
import { UnionResultSet } from './query/resultset/UnionResultSet';
import { FilteringResultSet } from './query/resultset/FilteringResultSet';
import { evaluatePredicate, PredicateNode } from './predicate';

// Adaptive indexing imports
import {
  QueryPatternTracker,
  IndexAdvisor,
  AutoIndexManager,
  DefaultIndexingStrategy,
} from './query/adaptive';
import type {
  IndexedMapOptions,
  IndexSuggestion,
  IndexSuggestionOptions,
  QueryStatistics,
  TrackedQueryType,
  RecommendedIndexType,
  IndexBuildProgressCallback,
} from './query/adaptive/types';
import { ADAPTIVE_INDEXING_DEFAULTS } from './query/adaptive/types';
import type { LazyIndex } from './query/indexes/lazy';

/**
 * LWWMap with index support for O(1) to O(log N) queries.
 *
 * K = key type (extends string for compatibility)
 * V = value type
 */
export class IndexedLWWMap<K extends string, V> extends LWWMap<K, V> {
  private indexRegistry: IndexRegistry<K, V>;
  private standingQueryRegistry: StandingQueryRegistry<K, V>;
  private liveQueryManager: LiveQueryManager<K, V>;
  private queryOptimizer: QueryOptimizer<K, V>;

  // Adaptive indexing
  private readonly queryTracker: QueryPatternTracker;
  private readonly indexAdvisor: IndexAdvisor;
  private readonly autoIndexManager: AutoIndexManager<K, V> | null;
  private readonly defaultIndexingStrategy: DefaultIndexingStrategy<V> | null;
  private readonly options: IndexedMapOptions;

  constructor(hlc: HLC, options: IndexedMapOptions = {}) {
    super(hlc);
    this.options = options;

    this.indexRegistry = new IndexRegistry();
    this.standingQueryRegistry = new StandingQueryRegistry({
      getRecord: (key) => this.get(key),
      getAllEntries: () => this.entries(),
    });
    this.liveQueryManager = new LiveQueryManager({
      getRecord: (key) => this.get(key),
      getAllEntries: () => this.entries(),
    });
    this.queryOptimizer = new QueryOptimizer({
      indexRegistry: this.indexRegistry,
      standingQueryRegistry: this.standingQueryRegistry,
    });

    // Set up fallback index for full scans
    this.indexRegistry.setFallbackIndex(
      new FallbackIndex<K, V>(
        () => this.keys(),
        (key) => this.get(key),
        (record, query) => this.matchesIndexQuery(record, query)
      )
    );

    // Initialize adaptive indexing
    this.queryTracker = new QueryPatternTracker();
    this.indexAdvisor = new IndexAdvisor(this.queryTracker);

    // Initialize auto-index manager if enabled
    if (options.adaptiveIndexing?.autoIndex?.enabled) {
      this.autoIndexManager = new AutoIndexManager(
        this.queryTracker,
        this.indexAdvisor,
        options.adaptiveIndexing.autoIndex
      );
      this.autoIndexManager.setMap(this);
    } else {
      this.autoIndexManager = null;
    }

    // Initialize default indexing strategy
    if (options.defaultIndexing && options.defaultIndexing !== 'none') {
      this.defaultIndexingStrategy = new DefaultIndexingStrategy<V>(options.defaultIndexing);
    } else {
      this.defaultIndexingStrategy = null;
    }
  }

  // ==================== Index Management ====================

  /**
   * Add a hash index on an attribute.
   * If lazyIndexBuilding is enabled, creates a LazyHashIndex instead.
   *
   * @param attribute - Attribute to index
   * @returns Created HashIndex (or LazyHashIndex)
   */
  addHashIndex<A>(attribute: Attribute<V, A>): HashIndex<K, V, A> | LazyHashIndex<K, V, A> {
    if (this.options.lazyIndexBuilding) {
      const index = new LazyHashIndex<K, V, A>(attribute, {
        onProgress: this.options.onIndexBuilding,
      });
      this.indexRegistry.addIndex(index);
      this.buildIndex(index);
      return index;
    }

    const index = new HashIndex<K, V, A>(attribute);
    this.indexRegistry.addIndex(index);
    this.buildIndex(index);
    return index;
  }

  /**
   * Add a navigable index on an attribute.
   * Navigable indexes support range queries (gt, gte, lt, lte, between).
   * If lazyIndexBuilding is enabled, creates a LazyNavigableIndex instead.
   *
   * @param attribute - Attribute to index
   * @param comparator - Optional custom comparator
   * @returns Created NavigableIndex (or LazyNavigableIndex)
   */
  addNavigableIndex<A extends string | number>(
    attribute: Attribute<V, A>,
    comparator?: (a: A, b: A) => number
  ): NavigableIndex<K, V, A> | LazyNavigableIndex<K, V, A> {
    if (this.options.lazyIndexBuilding) {
      const index = new LazyNavigableIndex<K, V, A>(attribute, comparator, {
        onProgress: this.options.onIndexBuilding,
      });
      this.indexRegistry.addIndex(index);
      this.buildIndex(index);
      return index;
    }

    const index = new NavigableIndex<K, V, A>(attribute, comparator);
    this.indexRegistry.addIndex(index);
    this.buildIndex(index);
    return index;
  }

  /**
   * Add an inverted index for full-text search on an attribute.
   * Inverted indexes support text search queries (contains, containsAll, containsAny).
   * If lazyIndexBuilding is enabled, creates a LazyInvertedIndex instead.
   *
   * @param attribute - Text attribute to index
   * @param pipeline - Optional custom tokenization pipeline
   * @returns Created InvertedIndex (or LazyInvertedIndex)
   *
   * @example
   * ```typescript
   * const nameAttr = simpleAttribute<Product, string>('name', p => p.name);
   * products.addInvertedIndex(nameAttr);
   *
   * // Search for products containing "wireless"
   * products.query({ type: 'contains', attribute: 'name', value: 'wireless' });
   * ```
   */
  addInvertedIndex<A extends string = string>(
    attribute: Attribute<V, A>,
    pipeline?: TokenizationPipeline
  ): InvertedIndex<K, V, A> | LazyInvertedIndex<K, V, A> {
    if (this.options.lazyIndexBuilding) {
      const index = new LazyInvertedIndex<K, V, A>(attribute, pipeline, {
        onProgress: this.options.onIndexBuilding,
      });
      this.indexRegistry.addIndex(index);
      this.buildIndex(index);
      return index;
    }

    const index = new InvertedIndex<K, V, A>(attribute, pipeline);
    this.indexRegistry.addIndex(index);
    this.buildIndex(index);
    return index;
  }

  /**
   * Add a custom index.
   *
   * @param index - Index to add
   */
  addIndex<A>(index: Index<K, V, A>): void {
    this.indexRegistry.addIndex(index);
    this.buildIndex(index);
  }

  /**
   * Remove an index.
   *
   * @param index - Index to remove
   * @returns true if index was found and removed
   */
  removeIndex<A>(index: Index<K, V, A>): boolean {
    return this.indexRegistry.removeIndex(index);
  }

  /**
   * Get all indexes.
   *
   * @returns Array of all indexes
   */
  getIndexes(): Index<K, V, unknown>[] {
    return this.indexRegistry.getAllIndexes();
  }

  /**
   * Check if an attribute is indexed.
   *
   * @param attributeName - Attribute name
   * @returns true if attribute has indexes
   */
  hasIndexOn(attributeName: string): boolean {
    return this.indexRegistry.hasIndex(attributeName);
  }

  /**
   * Build index from existing data.
   */
  private buildIndex<A>(index: Index<K, V, A>): void {
    for (const [key, value] of this.entries()) {
      index.add(key, value);
    }
  }

  // ==================== Query Execution ====================

  /**
   * Execute a query using indexes.
   * Returns lazy ResultSet of matching keys.
   *
   * Also tracks query patterns for adaptive indexing.
   *
   * @param query - Query to execute
   * @returns ResultSet of matching keys
   */
  query(query: Query): ResultSet<K> {
    const start = performance.now();
    const plan = this.queryOptimizer.optimize(query);
    const resultSet = this.executePlan(plan.root);

    // Materialize for statistics
    const results = resultSet.toArray();
    const duration = performance.now() - start;

    // Track query pattern for adaptive indexing
    this.trackQueryPattern(query, duration, results.length, plan.usesIndexes);

    // Return a new SetResultSet with the materialized results
    return new SetResultSet(new Set(results), resultSet.getRetrievalCost());
  }

  /**
   * Execute a query and return materialized results.
   * Returns array of [key, value] pairs.
   *
   * @param query - Query to execute
   * @returns Array of [key, value] pairs
   */
  queryEntries(query: Query): [K, V][] {
    const keys = this.query(query);
    const results: [K, V][] = [];

    for (const key of keys) {
      const value = this.get(key);
      if (value !== undefined) {
        results.push([key, value]);
      }
    }

    return results;
  }

  /**
   * Execute a query and return matching values.
   *
   * @param query - Query to execute
   * @returns Array of matching values
   */
  queryValues(query: Query): V[] {
    const keys = this.query(query);
    const results: V[] = [];

    for (const key of keys) {
      const value = this.get(key);
      if (value !== undefined) {
        results.push(value);
      }
    }

    return results;
  }

  /**
   * Count matching records without materializing results.
   *
   * @param query - Query to execute
   * @returns Number of matching records
   */
  count(query: Query): number {
    const resultSet = this.query(query);
    return resultSet.size();
  }

  /**
   * Execute plan and return result set.
   */
  private executePlan(step: PlanStep): ResultSet<K> {
    switch (step.type) {
      case 'point-lookup': {
        const key = step.key as K;
        const result = new Set<K>();
        if (this.get(key) !== undefined) {
          result.add(key);
        }
        return new SetResultSet(result, 1);
      }

      case 'multi-point-lookup': {
        const result = new Set<K>();
        for (const key of step.keys) {
          const k = key as K;
          if (this.get(k) !== undefined) {
            result.add(k);
          }
        }
        return new SetResultSet(result, step.keys.length);
      }

      case 'index-scan':
        return step.index.retrieve(step.query) as ResultSet<K>;

      case 'full-scan': {
        const fallback = this.indexRegistry.getFallbackIndex();
        if (fallback) {
          // FallbackIndex uses predicate internally - cast through unknown for compatibility
          return fallback.retrieve(step.predicate as unknown as IndexQuery<unknown>) as ResultSet<K>;
        }
        // Manual full scan fallback
        return this.fullScan(step.predicate as Query);
      }

      case 'intersection':
        return new IntersectionResultSet(
          step.steps.map((s) => this.executePlan(s))
        );

      case 'union':
        return new UnionResultSet(step.steps.map((s) => this.executePlan(s)));

      case 'filter':
        return new FilteringResultSet(
          this.executePlan(step.source),
          (key) => this.get(key),
          (record) => {
            if (record === undefined) return false;
            return this.matchesPredicate(record, step.predicate as Query);
          }
        );

      case 'not': {
        const matching = new Set(this.executePlan(step.source).toArray());
        const allKeysSet = new Set(this.keys());
        for (const key of matching) {
          allKeysSet.delete(key);
        }
        return new SetResultSet(allKeysSet, 100);
      }

      default:
        throw new Error(`Unknown plan step type: ${(step as PlanStep).type}`);
    }
  }

  /**
   * Perform full scan with predicate evaluation.
   */
  private fullScan(query: Query): ResultSet<K> {
    const result = new Set<K>();
    for (const [key, value] of this.entries()) {
      if (this.matchesPredicate(value, query)) {
        result.add(key);
      }
    }
    return new SetResultSet(result, Number.MAX_SAFE_INTEGER);
  }

  /**
   * Check if record matches predicate.
   * Converts Query to PredicateNode format for evaluation.
   */
  private matchesPredicate(record: V, query: Query): boolean {
    try {
      const predicate = this.queryToPredicate(query);
      return evaluatePredicate(predicate, record);
    } catch {
      return false;
    }
  }

  /**
   * Check if record matches IndexQuery (used by FallbackIndex).
   * This is a simplified matcher for full scan fallback.
   */
  private matchesIndexQuery(record: V, query: IndexQuery<unknown>): boolean {
    // Full scan matcher - evaluates the stored predicate
    // FallbackIndex passes the original query predicate
    if ('attribute' in (query as unknown as Record<string, unknown>)) {
      // This is a Query-like object passed through
      return this.matchesPredicate(record, query as unknown as Query);
    }
    // For simple IndexQuery without attribute context, always match
    return true;
  }

  /**
   * Convert Query to PredicateNode format.
   */
  private queryToPredicate(query: Query): PredicateNode {
    if ('type' in query) {
      switch (query.type) {
        case 'eq':
          return {
            op: 'eq',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { value: unknown }).value,
          };
        case 'neq':
          return {
            op: 'neq',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { value: unknown }).value,
          };
        case 'gt':
          return {
            op: 'gt',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { value: unknown }).value,
          };
        case 'gte':
          return {
            op: 'gte',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { value: unknown }).value,
          };
        case 'lt':
          return {
            op: 'lt',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { value: unknown }).value,
          };
        case 'lte':
          return {
            op: 'lte',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { value: unknown }).value,
          };
        case 'and':
          return {
            op: 'and',
            children: ((query as { children: Query[] }).children || []).map(
              (c) => this.queryToPredicate(c)
            ),
          };
        case 'or':
          return {
            op: 'or',
            children: ((query as { children: Query[] }).children || []).map(
              (c) => this.queryToPredicate(c)
            ),
          };
        case 'not':
          return {
            op: 'not',
            children: [
              this.queryToPredicate((query as { child: Query }).child),
            ],
          };
        case 'contains':
          return {
            op: 'contains',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { value: unknown }).value,
          };
        case 'containsAll':
          return {
            op: 'containsAll',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { values: unknown[] }).values,
          };
        case 'containsAny':
          return {
            op: 'containsAny',
            attribute: (query as { attribute: string }).attribute,
            value: (query as { values: unknown[] }).values,
          };
        default:
          return { op: 'eq', value: null };
      }
    }
    return { op: 'eq', value: null };
  }

  // ==================== Live Queries ====================

  /**
   * Subscribe to a live query.
   * Callback receives initial results and delta updates.
   *
   * @param query - Query to subscribe to
   * @param callback - Callback for query events
   * @returns Unsubscribe function
   */
  subscribeLiveQuery(
    query: Query,
    callback: LiveQueryCallback<K, V>
  ): () => void {
    return this.liveQueryManager.subscribe(query, callback);
  }

  /**
   * Get current live query results (snapshot).
   *
   * @param query - Query to execute
   * @returns Array of matching keys
   */
  getLiveQueryResults(query: Query): K[] {
    return this.liveQueryManager.getResults(query);
  }

  /**
   * Check if a query has active subscribers.
   *
   * @param query - Query to check
   * @returns true if query has subscribers
   */
  hasLiveQuerySubscribers(query: Query): boolean {
    return this.liveQueryManager.hasSubscribers(query);
  }

  // ==================== Override CRDT Operations ====================

  /**
   * Set a value (with index updates).
   */
  public set(key: K, value: V, ttlMs?: number): LWWRecord<V> {
    const oldValue = this.get(key);
    const result = super.set(key, value, ttlMs);

    // Apply default indexing on first record
    if (this.defaultIndexingStrategy && !this.defaultIndexingStrategy.isApplied()) {
      this.defaultIndexingStrategy.applyToMap(this, value);
    }

    if (oldValue !== undefined) {
      this.indexRegistry.onRecordUpdated(key, oldValue, value);
      this.liveQueryManager.onRecordUpdated(key, oldValue, value);
    } else {
      this.indexRegistry.onRecordAdded(key, value);
      this.liveQueryManager.onRecordAdded(key, value);
    }

    return result;
  }

  /**
   * Remove a value (with index updates).
   */
  public remove(key: K): LWWRecord<V> {
    const oldValue = this.get(key);
    const result = super.remove(key);

    if (oldValue !== undefined) {
      this.indexRegistry.onRecordRemoved(key, oldValue);
      this.liveQueryManager.onRecordRemoved(key, oldValue);
    }

    return result;
  }

  /**
   * Merge a remote record (with index updates).
   */
  public merge(key: K, remote: LWWRecord<V>): boolean {
    const oldValue = this.get(key);
    const merged = super.merge(key, remote);

    if (merged) {
      const newValue = this.get(key);

      if (oldValue === undefined && newValue !== undefined) {
        // New record
        this.indexRegistry.onRecordAdded(key, newValue);
        this.liveQueryManager.onRecordAdded(key, newValue);
      } else if (oldValue !== undefined && newValue === undefined) {
        // Deleted (tombstone)
        this.indexRegistry.onRecordRemoved(key, oldValue);
        this.liveQueryManager.onRecordRemoved(key, oldValue);
      } else if (oldValue !== undefined && newValue !== undefined) {
        // Updated
        this.indexRegistry.onRecordUpdated(key, oldValue, newValue);
        this.liveQueryManager.onRecordUpdated(key, oldValue, newValue);
      }
    }

    return merged;
  }

  /**
   * Clear all data (and indexes).
   */
  public clear(): void {
    super.clear();
    this.indexRegistry.clear();
    this.liveQueryManager.clear();
  }

  // ==================== Iterator Methods ====================

  /**
   * Returns all keys (non-tombstoned, non-expired).
   */
  public keys(): Iterable<K> {
    const self = this;
    return {
      *[Symbol.iterator]() {
        for (const [key] of self.entries()) {
          yield key;
        }
      },
    };
  }

  // ==================== Stats ====================

  /**
   * Get index statistics.
   */
  getIndexStats(): Map<string, IndexStats> {
    const stats = new Map<string, IndexStats>();
    for (const index of this.indexRegistry.getAllIndexes()) {
      stats.set(index.attribute.name, index.getStats());
    }
    return stats;
  }

  /**
   * Get index registry statistics.
   */
  getIndexRegistryStats(): IndexRegistryStats {
    return this.indexRegistry.getStats();
  }

  /**
   * Get query optimizer for plan inspection.
   */
  getQueryOptimizer(): QueryOptimizer<K, V> {
    return this.queryOptimizer;
  }

  /**
   * Get live query manager for direct access.
   */
  getLiveQueryManager(): LiveQueryManager<K, V> {
    return this.liveQueryManager;
  }

  /**
   * Get standing query registry for direct access.
   */
  getStandingQueryRegistry(): StandingQueryRegistry<K, V> {
    return this.standingQueryRegistry;
  }

  /**
   * Explain query execution plan.
   *
   * @param query - Query to explain
   * @returns Query execution plan
   */
  explainQuery(query: Query): QueryPlan {
    return this.queryOptimizer.optimize(query);
  }

  // ==================== Adaptive Indexing ====================

  /**
   * Register an attribute for auto-indexing.
   * Required before auto-index can create indexes on this attribute.
   *
   * @param attribute - The attribute to register
   * @param allowedIndexTypes - Optional list of allowed index types
   */
  registerAttribute<A>(
    attribute: Attribute<V, A>,
    allowedIndexTypes?: RecommendedIndexType[]
  ): void {
    if (this.autoIndexManager) {
      this.autoIndexManager.registerAttribute(attribute, allowedIndexTypes);
    }
  }

  /**
   * Unregister an attribute from auto-indexing.
   *
   * @param attributeName - Name of attribute to unregister
   */
  unregisterAttribute(attributeName: string): void {
    if (this.autoIndexManager) {
      this.autoIndexManager.unregisterAttribute(attributeName);
    }
  }

  /**
   * Get index suggestions based on query patterns.
   * Use this in production to get recommendations for manual index creation.
   *
   * @param options - Suggestion options
   * @returns Array of index suggestions sorted by priority
   *
   * @example
   * ```typescript
   * const suggestions = products.getIndexSuggestions();
   * // [{ attribute: 'category', indexType: 'hash', priority: 'high', ... }]
   * ```
   */
  getIndexSuggestions(options?: IndexSuggestionOptions): IndexSuggestion[] {
    return this.indexAdvisor.getSuggestions(options);
  }

  /**
   * Get query pattern statistics.
   * Useful for debugging and understanding query patterns.
   *
   * @returns Array of query statistics
   */
  getQueryStatistics(): QueryStatistics[] {
    return this.queryTracker.getStatistics();
  }

  /**
   * Reset query statistics.
   * Call this to clear accumulated query patterns.
   */
  resetQueryStatistics(): void {
    this.queryTracker.clear();
    if (this.autoIndexManager) {
      this.autoIndexManager.resetCounts();
    }
  }

  /**
   * Get query pattern tracker for advanced usage.
   */
  getQueryTracker(): QueryPatternTracker {
    return this.queryTracker;
  }

  /**
   * Get index advisor for advanced usage.
   */
  getIndexAdvisor(): IndexAdvisor {
    return this.indexAdvisor;
  }

  /**
   * Get auto-index manager (if enabled).
   */
  getAutoIndexManager(): AutoIndexManager<K, V> | null {
    return this.autoIndexManager;
  }

  /**
   * Check if auto-indexing is enabled.
   */
  isAutoIndexingEnabled(): boolean {
    return this.autoIndexManager !== null;
  }

  // ==================== Lazy Indexing ====================

  /**
   * Check if lazy index building is enabled.
   */
  isLazyIndexingEnabled(): boolean {
    return this.options.lazyIndexBuilding === true;
  }

  /**
   * Force materialization of all lazy indexes.
   * Useful to pre-warm indexes before critical operations.
   *
   * @param progressCallback - Optional progress callback
   */
  materializeAllIndexes(progressCallback?: IndexBuildProgressCallback): void {
    const callback = progressCallback ?? this.options.onIndexBuilding;

    for (const index of this.indexRegistry.getAllIndexes()) {
      if ('isLazy' in index && (index as LazyIndex<K, V, unknown>).isLazy) {
        const lazyIndex = index as LazyIndex<K, V, unknown>;
        if (!lazyIndex.isBuilt) {
          lazyIndex.materialize(callback);
        }
      }
    }
  }

  /**
   * Get count of pending records across all lazy indexes.
   * Returns 0 if no lazy indexes or all are materialized.
   */
  getPendingIndexCount(): number {
    let total = 0;
    for (const index of this.indexRegistry.getAllIndexes()) {
      if ('isLazy' in index && (index as LazyIndex<K, V, unknown>).isLazy) {
        const lazyIndex = index as LazyIndex<K, V, unknown>;
        total += lazyIndex.pendingCount;
      }
    }
    return total;
  }

  /**
   * Check if any lazy indexes are still pending (not built).
   */
  hasUnbuiltIndexes(): boolean {
    for (const index of this.indexRegistry.getAllIndexes()) {
      if ('isLazy' in index && (index as LazyIndex<K, V, unknown>).isLazy) {
        const lazyIndex = index as LazyIndex<K, V, unknown>;
        if (!lazyIndex.isBuilt) {
          return true;
        }
      }
    }
    return false;
  }

  /**
   * Track query pattern for adaptive indexing.
   */
  private trackQueryPattern(
    query: Query,
    duration: number,
    resultSize: number,
    usedIndex: boolean
  ): void {
    // Only track if advisor is enabled (default: true)
    const advisorEnabled = this.options.adaptiveIndexing?.advisor?.enabled ??
      ADAPTIVE_INDEXING_DEFAULTS.advisor.enabled;

    if (!advisorEnabled && !this.autoIndexManager) {
      return;
    }

    // Extract attribute from query
    const attribute = this.extractAttribute(query);
    if (!attribute) return;

    // Extract query type
    const queryType = this.extractQueryType(query);
    if (!queryType) return;

    // Check if this attribute has an index
    const hasIndex = this.indexRegistry.hasIndex(attribute);

    // Record query in tracker
    this.queryTracker.recordQuery(
      attribute,
      queryType,
      duration,
      resultSize,
      hasIndex
    );

    // Notify auto-index manager if enabled
    if (this.autoIndexManager) {
      this.autoIndexManager.onQueryExecuted(attribute, queryType);
    }
  }

  /**
   * Extract attribute name from query.
   */
  private extractAttribute(query: Query): string | null {
    if (isSimpleQuery(query)) {
      return (query as SimpleQueryNode).attribute;
    }

    // For compound queries, extract from first child
    if (query.type === 'and' || query.type === 'or') {
      const children = (query as { children?: Query[] }).children;
      if (children && children.length > 0) {
        return this.extractAttribute(children[0]);
      }
    }

    if (query.type === 'not') {
      const child = (query as { child?: Query }).child;
      if (child) {
        return this.extractAttribute(child);
      }
    }

    return null;
  }

  /**
   * Extract query type from query.
   */
  private extractQueryType(query: Query): TrackedQueryType | null {
    if (isSimpleQuery(query)) {
      const type = query.type;
      // Only track types that can be indexed
      const indexableTypes: TrackedQueryType[] = [
        'eq', 'neq', 'gt', 'gte', 'lt', 'lte', 'between', 'in', 'has',
        'contains', 'containsAll', 'containsAny',
      ];
      if (indexableTypes.includes(type as TrackedQueryType)) {
        return type as TrackedQueryType;
      }
    }

    // For compound queries, extract from first child
    if (query.type === 'and' || query.type === 'or') {
      const children = (query as { children?: Query[] }).children;
      if (children && children.length > 0) {
        return this.extractQueryType(children[0]);
      }
    }

    if (query.type === 'not') {
      const child = (query as { child?: Query }).child;
      if (child) {
        return this.extractQueryType(child);
      }
    }

    return null;
  }
}
