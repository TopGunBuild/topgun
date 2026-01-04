/**
 * IndexedORMap Implementation
 *
 * ORMap with index support for O(1) to O(log N) queries.
 * Wraps ORMap with indexing capabilities using the Wrapper Pattern.
 *
 * Note: ORMap stores multiple values per key (with tags).
 * Indexes track unique (key, tag) composite keys.
 *
 * Features:
 * - Hash and Navigable indexes for efficient queries
 * - Composite key indexing (key:tag)
 * - Automatic index updates on CRDT operations
 * - Lazy filtering for tombstones
 * - Adaptive indexing with query pattern tracking (Phase 8.02)
 *
 * @module IndexedORMap
 */

import { ORMap, ORMapRecord } from './ORMap';
import { HLC, Timestamp } from './HLC';
import { IndexRegistry, IndexRegistryStats } from './query/IndexRegistry';
import { QueryOptimizer } from './query/QueryOptimizer';
import type { Index, IndexStats, IndexQuery } from './query/indexes/types';
import { HashIndex } from './query/indexes/HashIndex';
import { NavigableIndex } from './query/indexes/NavigableIndex';
import { FallbackIndex } from './query/indexes/FallbackIndex';
import { InvertedIndex } from './query/indexes/InvertedIndex';
import { TokenizationPipeline } from './query/tokenization';
import { Attribute, simpleAttribute } from './query/Attribute';
import type { Query, QueryPlan, PlanStep, SimpleQueryNode } from './query/QueryTypes';
import { isSimpleQuery } from './query/QueryTypes';
import type { ResultSet } from './query/resultset/ResultSet';
import { SetResultSet } from './query/resultset/SetResultSet';
import { IntersectionResultSet } from './query/resultset/IntersectionResultSet';
import { UnionResultSet } from './query/resultset/UnionResultSet';
import { FilteringResultSet } from './query/resultset/FilteringResultSet';
import { evaluatePredicate, PredicateNode } from './predicate';

// Full-Text Search imports (Phase 11)
import { FullTextIndex } from './fts/FullTextIndex';
import type { FullTextIndexConfig, SearchOptions as FTSSearchOptions, ScoredDocument } from './fts/types';

// Adaptive indexing imports (Phase 8.02)
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
} from './query/adaptive/types';
import { ADAPTIVE_INDEXING_DEFAULTS } from './query/adaptive/types';

/**
 * Result of a query on IndexedORMap.
 */
export interface ORMapQueryResult<K, V> {
  key: K;
  tag: string;
  value: V;
}

/**
 * Result of a full-text search on IndexedORMap.
 * Includes BM25 relevance score for ranking.
 */
export interface ORMapSearchResult<K, V> extends ORMapQueryResult<K, V> {
  /** BM25 relevance score */
  score: number;
  /** Terms from the query that matched */
  matchedTerms: string[];
}

/**
 * ORMap with index support.
 *
 * Note: ORMap stores multiple values per key (with tags).
 * Indexes track unique (key, tag) pairs using composite keys.
 *
 * K = key type (extends string for compatibility)
 * V = value type
 */
export class IndexedORMap<K extends string, V> extends ORMap<K, V> {
  // Composite key = "mapKey:tag"
  private indexRegistry: IndexRegistry<string, V>;
  private queryOptimizer: QueryOptimizer<string, V>;

  // Adaptive indexing (Phase 8.02)
  private readonly queryTracker: QueryPatternTracker;
  private readonly indexAdvisor: IndexAdvisor;
  private readonly autoIndexManager: AutoIndexManager<string, V> | null;
  private readonly defaultIndexingStrategy: DefaultIndexingStrategy<V> | null;
  private readonly options: IndexedMapOptions;

  // Full-Text Search (Phase 11)
  private fullTextIndex: FullTextIndex | null = null;

  constructor(hlc: HLC, options: IndexedMapOptions = {}) {
    super(hlc);
    this.options = options;

    this.indexRegistry = new IndexRegistry();
    this.queryOptimizer = new QueryOptimizer({
      indexRegistry: this.indexRegistry,
    });

    // Set up fallback index for full scans
    this.indexRegistry.setFallbackIndex(
      new FallbackIndex<string, V>(
        () => this.getAllCompositeKeys(),
        (compositeKey) => this.getRecordByCompositeKey(compositeKey),
        (record, query) => this.matchesIndexQuery(record, query)
      )
    );

    // Initialize adaptive indexing (Phase 8.02)
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
   *
   * @param attribute - Attribute to index
   * @returns Created HashIndex
   */
  addHashIndex<A>(attribute: Attribute<V, A>): HashIndex<string, V, A> {
    const index = new HashIndex<string, V, A>(attribute);
    this.indexRegistry.addIndex(index);
    this.buildIndexFromExisting(index);
    return index;
  }

  /**
   * Add a navigable index on an attribute.
   * Navigable indexes support range queries (gt, gte, lt, lte, between).
   *
   * @param attribute - Attribute to index
   * @param comparator - Optional custom comparator
   * @returns Created NavigableIndex
   */
  addNavigableIndex<A extends string | number>(
    attribute: Attribute<V, A>,
    comparator?: (a: A, b: A) => number
  ): NavigableIndex<string, V, A> {
    const index = new NavigableIndex<string, V, A>(attribute, comparator);
    this.indexRegistry.addIndex(index);
    this.buildIndexFromExisting(index);
    return index;
  }

  /**
   * Add an inverted index for full-text search on an attribute.
   * Inverted indexes support text search queries (contains, containsAll, containsAny).
   *
   * @param attribute - Text attribute to index
   * @param pipeline - Optional custom tokenization pipeline
   * @returns Created InvertedIndex
   */
  addInvertedIndex<A extends string = string>(
    attribute: Attribute<V, A>,
    pipeline?: TokenizationPipeline
  ): InvertedIndex<string, V, A> {
    const index = new InvertedIndex<string, V, A>(attribute, pipeline);
    this.indexRegistry.addIndex(index);
    this.buildIndexFromExisting(index);
    return index;
  }

  /**
   * Add a custom index.
   *
   * @param index - Index to add
   */
  addIndex<A>(index: Index<string, V, A>): void {
    this.indexRegistry.addIndex(index);
    this.buildIndexFromExisting(index);
  }

  // ==================== Full-Text Search (Phase 11) ====================

  /**
   * Enable BM25-based full-text search on specified fields.
   * This creates a FullTextIndex for relevance-ranked search.
   *
   * Note: This is different from addInvertedIndex which provides
   * boolean matching (contains/containsAll/containsAny). This method
   * provides BM25 relevance scoring for true full-text search.
   *
   * @param config - Full-text index configuration
   * @returns The created FullTextIndex
   *
   * @example
   * ```typescript
   * const map = new IndexedORMap(hlc);
   * map.enableFullTextSearch({
   *   fields: ['title', 'body'],
   *   tokenizer: { minLength: 2 },
   *   bm25: { k1: 1.2, b: 0.75 }
   * });
   *
   * map.add('doc1', { title: 'Hello World', body: 'Test content' });
   * const results = map.search('hello');
   * // [{ key: 'doc1', tag: '...', value: {...}, score: 0.5, matchedTerms: ['hello'] }]
   * ```
   */
  enableFullTextSearch(config: FullTextIndexConfig): FullTextIndex {
    // Create the full-text index
    this.fullTextIndex = new FullTextIndex(config);

    // Build from existing data
    const snapshot = this.getSnapshot();
    const entries: Array<[string, Record<string, unknown>]> = [];

    for (const [key, tagMap] of snapshot.items) {
      for (const [tag, record] of tagMap) {
        if (!snapshot.tombstones.has(tag)) {
          const compositeKey = this.createCompositeKey(key, tag);
          entries.push([compositeKey, record.value as Record<string, unknown>]);
        }
      }
    }

    this.fullTextIndex.buildFromEntries(entries);

    return this.fullTextIndex;
  }

  /**
   * Check if full-text search is enabled.
   *
   * @returns true if full-text search is enabled
   */
  isFullTextSearchEnabled(): boolean {
    return this.fullTextIndex !== null;
  }

  /**
   * Get the full-text index (if enabled).
   *
   * @returns The FullTextIndex or null
   */
  getFullTextIndex(): FullTextIndex | null {
    return this.fullTextIndex;
  }

  /**
   * Perform a BM25-ranked full-text search.
   * Results are sorted by relevance score (highest first).
   *
   * @param query - Search query text
   * @param options - Search options (limit, minScore, boost)
   * @returns Array of search results with scores, sorted by relevance
   *
   * @throws Error if full-text search is not enabled
   */
  search(query: string, options?: FTSSearchOptions): ORMapSearchResult<K, V>[] {
    if (!this.fullTextIndex) {
      throw new Error('Full-text search is not enabled. Call enableFullTextSearch() first.');
    }

    const scoredDocs = this.fullTextIndex.search(query, options);
    const results: ORMapSearchResult<K, V>[] = [];

    for (const { docId: compositeKey, score, matchedTerms } of scoredDocs) {
      const [key, tag] = this.parseCompositeKey(compositeKey);
      const records = this.getRecords(key as K);
      const record = records.find((r) => r.tag === tag);

      if (record) {
        results.push({
          key: key as K,
          tag,
          value: record.value,
          score,
          matchedTerms: matchedTerms ?? [],
        });
      }
    }

    return results;
  }

  /**
   * Disable full-text search and release the index.
   */
  disableFullTextSearch(): void {
    if (this.fullTextIndex) {
      this.fullTextIndex.clear();
      this.fullTextIndex = null;
    }
  }

  /**
   * Remove an index.
   *
   * @param index - Index to remove
   * @returns true if index was found and removed
   */
  removeIndex<A>(index: Index<string, V, A>): boolean {
    return this.indexRegistry.removeIndex(index);
  }

  /**
   * Get all indexes.
   *
   * @returns Array of all indexes
   */
  getIndexes(): Index<string, V, unknown>[] {
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
  private buildIndexFromExisting<A>(index: Index<string, V, A>): void {
    const snapshot = this.getSnapshot();
    for (const [key, tagMap] of snapshot.items) {
      for (const [tag, record] of tagMap) {
        if (!snapshot.tombstones.has(tag)) {
          const compositeKey = this.createCompositeKey(key, tag);
          index.add(compositeKey, record.value);
        }
      }
    }
  }

  // ==================== Query Execution ====================

  /**
   * Execute a query across all records.
   * Returns array of matching results with key, tag, and value.
   *
   * Also tracks query patterns for adaptive indexing (Phase 8.02).
   *
   * @param query - Query to execute
   * @returns Array of query results
   */
  query(query: Query): ORMapQueryResult<K, V>[] {
    const start = performance.now();
    const plan = this.queryOptimizer.optimize(query);
    const resultSet = this.executePlan(plan.root);

    const results: ORMapQueryResult<K, V>[] = [];
    for (const compositeKey of resultSet) {
      const [key, tag] = this.parseCompositeKey(compositeKey);
      const records = this.getRecords(key as K);
      const record = records.find((r) => r.tag === tag);
      if (record) {
        results.push({ key: key as K, tag, value: record.value });
      }
    }

    // Track query pattern for adaptive indexing (Phase 8.02)
    const duration = performance.now() - start;
    this.trackQueryPattern(query, duration, results.length, plan.usesIndexes);

    return results;
  }

  /**
   * Execute a query and return matching values only.
   *
   * @param query - Query to execute
   * @returns Array of matching values
   */
  queryValues(query: Query): V[] {
    return this.query(query).map((r) => r.value);
  }

  /**
   * Count matching records without materializing results.
   *
   * @param query - Query to execute
   * @returns Number of matching records
   */
  count(query: Query): number {
    const plan = this.queryOptimizer.optimize(query);
    const resultSet = this.executePlan(plan.root);
    return resultSet.size();
  }

  /**
   * Execute plan and return result set.
   */
  private executePlan(step: PlanStep): ResultSet<string> {
    switch (step.type) {
      case 'index-scan':
        return step.index.retrieve(step.query) as ResultSet<string>;

      case 'full-scan': {
        const fallback = this.indexRegistry.getFallbackIndex();
        if (fallback) {
          // FallbackIndex uses predicate internally - cast through unknown for compatibility
          return fallback.retrieve(step.predicate as unknown as IndexQuery<unknown>) as ResultSet<string>;
        }
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
          (compositeKey) => this.getRecordByCompositeKey(compositeKey),
          (record) => {
            if (record === undefined) return false;
            return this.matchesPredicate(record, step.predicate as Query);
          }
        );

      case 'not': {
        const matching = new Set(this.executePlan(step.source).toArray());
        const allKeysSet = new Set(this.getAllCompositeKeys());
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
  private fullScan(query: Query): ResultSet<string> {
    const result = new Set<string>();
    const snapshot = this.getSnapshot();

    for (const [key, tagMap] of snapshot.items) {
      for (const [tag, record] of tagMap) {
        if (!snapshot.tombstones.has(tag)) {
          if (this.matchesPredicate(record.value, query)) {
            result.add(this.createCompositeKey(key, tag));
          }
        }
      }
    }

    return new SetResultSet(result, Number.MAX_SAFE_INTEGER);
  }

  // ==================== Override CRDT Operations ====================

  /**
   * Add a value (with index updates).
   */
  public add(key: K, value: V, ttlMs?: number): ORMapRecord<V> {
    const record = super.add(key, value, ttlMs);
    const compositeKey = this.createCompositeKey(key, record.tag);
    this.indexRegistry.onRecordAdded(compositeKey, value);

    // Update full-text index (Phase 11)
    if (this.fullTextIndex) {
      this.fullTextIndex.onSet(compositeKey, value as Record<string, unknown>);
    }

    return record;
  }

  /**
   * Remove a value (with index updates).
   */
  public remove(key: K, value: V): string[] {
    const records = this.getRecords(key);
    const matchingRecords = records.filter((r) => r.value === value);
    const result = super.remove(key, value);

    for (const record of matchingRecords) {
      const compositeKey = this.createCompositeKey(key, record.tag);
      this.indexRegistry.onRecordRemoved(compositeKey, record.value);

      // Update full-text index (Phase 11)
      if (this.fullTextIndex) {
        this.fullTextIndex.onRemove(compositeKey);
      }
    }

    return result;
  }

  /**
   * Apply a record from remote (with index updates).
   */
  public apply(key: K, record: ORMapRecord<V>): boolean {
    const applied = super.apply(key, record);
    if (applied) {
      const compositeKey = this.createCompositeKey(key, record.tag);
      this.indexRegistry.onRecordAdded(compositeKey, record.value);

      // Update full-text index (Phase 11)
      if (this.fullTextIndex) {
        this.fullTextIndex.onSet(compositeKey, record.value as Record<string, unknown>);
      }
    }
    return applied;
  }

  /**
   * Apply a tombstone (with index updates).
   */
  public applyTombstone(tag: string): void {
    // Find the record before tombstoning
    const snapshot = this.getSnapshot();
    let removedValue: V | undefined;
    let removedKey: K | undefined;

    for (const [key, tagMap] of snapshot.items) {
      const record = tagMap.get(tag);
      if (record) {
        removedValue = record.value;
        removedKey = key;
        break;
      }
    }

    super.applyTombstone(tag);

    if (removedValue !== undefined && removedKey !== undefined) {
      const compositeKey = this.createCompositeKey(removedKey, tag);
      this.indexRegistry.onRecordRemoved(compositeKey, removedValue);

      // Update full-text index (Phase 11)
      if (this.fullTextIndex) {
        this.fullTextIndex.onRemove(compositeKey);
      }
    }
  }

  /**
   * Clear all data (and indexes).
   */
  public clear(): void {
    super.clear();
    this.indexRegistry.clear();

    // Clear full-text index (Phase 11)
    if (this.fullTextIndex) {
      this.fullTextIndex.clear();
    }
  }

  // ==================== Helper Methods ====================

  /**
   * Create composite key from map key and tag.
   * Uses '||' as separator since HLC tags contain ':'
   */
  private createCompositeKey(key: K, tag: string): string {
    return `${key}||${tag}`;
  }

  /**
   * Parse composite key into [key, tag].
   * Expects '||' separator.
   */
  private parseCompositeKey(compositeKey: string): [string, string] {
    const separatorIndex = compositeKey.indexOf('||');
    if (separatorIndex === -1) {
      // Fallback for malformed keys
      return [compositeKey, ''];
    }
    return [
      compositeKey.substring(0, separatorIndex),
      compositeKey.substring(separatorIndex + 2),
    ];
  }

  /**
   * Get all composite keys from the map.
   */
  private getAllCompositeKeys(): Iterable<string> {
    const self = this;
    return {
      *[Symbol.iterator]() {
        const snapshot = self.getSnapshot();
        for (const [key, tagMap] of snapshot.items) {
          for (const [tag] of tagMap) {
            if (!snapshot.tombstones.has(tag)) {
              yield self.createCompositeKey(key, tag);
            }
          }
        }
      },
    };
  }

  /**
   * Get record by composite key.
   */
  private getRecordByCompositeKey(compositeKey: string): V | undefined {
    const [key, tag] = this.parseCompositeKey(compositeKey);
    const records = this.getRecords(key as K);
    const record = records.find((r) => r.tag === tag);
    return record?.value;
  }

  /**
   * Check if record matches predicate.
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
  getQueryOptimizer(): QueryOptimizer<string, V> {
    return this.queryOptimizer;
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

  // ==================== Adaptive Indexing (Phase 8.02) ====================

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
  getAutoIndexManager(): AutoIndexManager<string, V> | null {
    return this.autoIndexManager;
  }

  /**
   * Check if auto-indexing is enabled.
   */
  isAutoIndexingEnabled(): boolean {
    return this.autoIndexManager !== null;
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
