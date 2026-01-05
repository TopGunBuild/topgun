/**
 * UnifiedLiveQueryRegistry Implementation (Phase 12)
 *
 * Unified registry for managing live query indexes.
 * Auto-detects index type based on query predicates:
 * - FTS predicates (match/matchPhrase/matchPrefix) → LiveFTSIndex
 * - All other predicates → StandingQueryIndex
 *
 * Features:
 * - Auto index type detection from query
 * - Reference counting for shared indexes
 * - Delta propagation to all affected indexes
 * - Support for hybrid queries (FTS + traditional predicates)
 *
 * @module query/UnifiedLiveQueryRegistry
 */

import { StandingQueryIndex, type StandingQueryChange } from './indexes/StandingQueryIndex';
import { LiveFTSIndex } from './indexes/LiveFTSIndex';
import type { LiveQueryDelta, ILiveQueryIndex, RankedResult } from './indexes/ILiveQueryIndex';
import type { FullTextIndex } from '../fts';
import type { Query, LogicalQueryNode, MatchQueryNode } from './QueryTypes';
import { isFTSQuery, isLogicalQuery, isMatchQuery } from './QueryTypes';

/**
 * Unified delta type that covers both binary and scored indexes.
 */
export interface UnifiedDelta<K> {
  /** Query hash this delta is for */
  queryHash: string;
  /** Query this delta is for */
  query: Query;
  /** Delta from the index */
  delta: LiveQueryDelta<K> | StandingQueryChange;
  /** Whether this is from an FTS index (has scores) */
  isFTS: boolean;
}

/**
 * Options for creating a UnifiedLiveQueryRegistry.
 */
export interface UnifiedLiveQueryRegistryOptions<K extends string, V> {
  /** Function to get record by key */
  getRecord: (key: K) => V | undefined;
  /** Function to get all entries for building index */
  getAllEntries: () => Iterable<[K, V]>;
  /** Optional FTS index for FTS queries */
  ftsIndex?: FullTextIndex;
}

/**
 * Statistics about the UnifiedLiveQueryRegistry.
 */
export interface UnifiedLiveQueryRegistryStats {
  /** Number of registered indexes */
  indexCount: number;
  /** Number of FTS indexes */
  ftsIndexCount: number;
  /** Number of standing query indexes */
  standingIndexCount: number;
  /** Total reference count across all indexes */
  totalRefCount: number;
  /** Total number of results across all indexes */
  totalResults: number;
}

/**
 * Internal entry for tracking an index.
 */
interface IndexEntry<K extends string, V> {
  /** The index (either StandingQueryIndex or LiveFTSIndex) */
  index: StandingQueryIndex<K, V> | LiveFTSIndex<K, V>;
  /** Whether this is an FTS index */
  isFTS: boolean;
  /** The original query */
  query: Query;
}

/**
 * Unified registry for managing live query indexes.
 * Automatically chooses index type based on query predicates.
 *
 * K = record key type (must extend string for FTS compatibility)
 * V = record value type
 */
export class UnifiedLiveQueryRegistry<K extends string, V> {
  /** Map from query hash to index entry */
  private readonly indexes: Map<string, IndexEntry<K, V>> = new Map();

  /** Reference count for each query */
  private readonly refCounts: Map<string, number> = new Map();

  /** Record accessor */
  private readonly getRecord: (key: K) => V | undefined;

  /** All entries accessor */
  private readonly getAllEntries: () => Iterable<[K, V]>;

  /** Optional FTS index for FTS queries */
  private readonly ftsIndex?: FullTextIndex;

  constructor(options: UnifiedLiveQueryRegistryOptions<K, V>) {
    this.getRecord = options.getRecord;
    this.getAllEntries = options.getAllEntries;
    this.ftsIndex = options.ftsIndex;
  }

  /**
   * Register a live query.
   * Auto-detects index type based on query predicates.
   * Increments reference count if query already registered.
   *
   * @param query - Query to register
   * @returns The index for the query
   */
  register(query: Query): StandingQueryIndex<K, V> | LiveFTSIndex<K, V> {
    const hash = this.hashQuery(query);

    // Check if already registered
    const existing = this.indexes.get(hash);
    if (existing) {
      this.refCounts.set(hash, (this.refCounts.get(hash) || 0) + 1);
      return existing.index;
    }

    // Determine index type based on query
    const ftsInfo = this.extractFTSInfo(query);
    // Only use FTS if we have both FTS info and an FTS index
    const canUseFTS = ftsInfo !== null && this.ftsIndex !== undefined;

    let index: StandingQueryIndex<K, V> | LiveFTSIndex<K, V>;

    if (canUseFTS) {
      // Create FTS index
      index = new LiveFTSIndex<K, V>(this.ftsIndex!, {
        field: ftsInfo.field,
        query: ftsInfo.query,
        minScore: ftsInfo.minScore,
        maxResults: ftsInfo.maxResults,
      });
      // Build from existing data
      index.buildFromData(this.getAllEntries());
    } else {
      // Create standing query index
      index = new StandingQueryIndex<K, V>({
        query,
        getRecord: this.getRecord,
      });
      // Build from existing data
      index.buildFromData(this.getAllEntries());
    }

    // Store in registry - isFTS reflects whether we actually used FTS
    this.indexes.set(hash, { index, isFTS: canUseFTS, query });
    this.refCounts.set(hash, 1);

    return index;
  }

  /**
   * Unregister a query.
   * Decrements reference count. Only removes when refcount reaches 0.
   *
   * @param query - Query to unregister
   * @returns true if index was removed, false if still has references
   */
  unregister(query: Query): boolean {
    const hash = this.hashQuery(query);
    const refCount = this.refCounts.get(hash) || 0;

    if (refCount <= 1) {
      const entry = this.indexes.get(hash);
      if (entry) {
        entry.index.clear();
      }
      this.indexes.delete(hash);
      this.refCounts.delete(hash);
      return true;
    }

    this.refCounts.set(hash, refCount - 1);
    return false;
  }

  /**
   * Get index for a query if registered.
   *
   * @param query - Query to look up
   * @returns Index or undefined if not registered
   */
  getIndex(query: Query): StandingQueryIndex<K, V> | LiveFTSIndex<K, V> | undefined {
    const hash = this.hashQuery(query);
    return this.indexes.get(hash)?.index;
  }

  /**
   * Get index by hash directly.
   *
   * @param hash - Query hash
   * @returns Index or undefined if not registered
   */
  getIndexByHash(hash: string): StandingQueryIndex<K, V> | LiveFTSIndex<K, V> | undefined {
    return this.indexes.get(hash)?.index;
  }

  /**
   * Check if query has an index registered.
   *
   * @param query - Query to check
   * @returns true if index exists
   */
  hasIndex(query: Query): boolean {
    const hash = this.hashQuery(query);
    return this.indexes.has(hash);
  }

  /**
   * Check if a query uses FTS index.
   *
   * @param query - Query to check
   * @returns true if query uses FTS index
   */
  isFTSIndex(query: Query): boolean {
    const hash = this.hashQuery(query);
    return this.indexes.get(hash)?.isFTS ?? false;
  }

  /**
   * Get reference count for a query.
   *
   * @param query - Query to check
   * @returns Reference count (0 if not registered)
   */
  getRefCount(query: Query): number {
    const hash = this.hashQuery(query);
    return this.refCounts.get(hash) || 0;
  }

  /**
   * Notify all indexes of record addition.
   * Returns unified deltas for affected queries.
   *
   * @param key - Record key
   * @param record - New record value
   * @returns Array of unified deltas
   */
  onRecordAdded(key: K, record: V): UnifiedDelta<K>[] {
    const deltas: UnifiedDelta<K>[] = [];

    for (const [hash, entry] of this.indexes) {
      if (entry.isFTS) {
        // FTS index
        const ftsIndex = entry.index as LiveFTSIndex<K, V>;
        const delta = ftsIndex.onRecordAdded(key, record);
        if (delta) {
          deltas.push({
            queryHash: hash,
            query: entry.query,
            delta,
            isFTS: true,
          });
        }
      } else {
        // Standing query index
        const sqIndex = entry.index as StandingQueryIndex<K, V>;
        const change = sqIndex.determineChange(key, undefined, record);
        if (change !== 'unchanged') {
          sqIndex.add(key, record);
          deltas.push({
            queryHash: hash,
            query: entry.query,
            delta: change,
            isFTS: false,
          });
        }
      }
    }

    return deltas;
  }

  /**
   * Notify all indexes of record update.
   * Returns unified deltas for affected queries.
   *
   * @param key - Record key
   * @param oldRecord - Previous record value
   * @param newRecord - New record value
   * @returns Array of unified deltas
   */
  onRecordUpdated(key: K, oldRecord: V, newRecord: V): UnifiedDelta<K>[] {
    const deltas: UnifiedDelta<K>[] = [];

    for (const [hash, entry] of this.indexes) {
      if (entry.isFTS) {
        // FTS index
        const ftsIndex = entry.index as LiveFTSIndex<K, V>;
        const delta = ftsIndex.onRecordUpdated(key, oldRecord, newRecord);
        if (delta) {
          deltas.push({
            queryHash: hash,
            query: entry.query,
            delta,
            isFTS: true,
          });
        }
      } else {
        // Standing query index
        const sqIndex = entry.index as StandingQueryIndex<K, V>;
        const change = sqIndex.determineChange(key, oldRecord, newRecord);
        if (change !== 'unchanged') {
          sqIndex.update(key, oldRecord, newRecord);
          deltas.push({
            queryHash: hash,
            query: entry.query,
            delta: change,
            isFTS: false,
          });
        }
      }
    }

    return deltas;
  }

  /**
   * Notify all indexes of record removal.
   * Returns unified deltas for affected queries.
   *
   * @param key - Record key
   * @param record - Removed record value
   * @returns Array of unified deltas
   */
  onRecordRemoved(key: K, record: V): UnifiedDelta<K>[] {
    const deltas: UnifiedDelta<K>[] = [];

    for (const [hash, entry] of this.indexes) {
      if (entry.isFTS) {
        // FTS index
        const ftsIndex = entry.index as LiveFTSIndex<K, V>;
        const delta = ftsIndex.onRecordRemoved(key, record);
        if (delta) {
          deltas.push({
            queryHash: hash,
            query: entry.query,
            delta,
            isFTS: true,
          });
        }
      } else {
        // Standing query index
        const sqIndex = entry.index as StandingQueryIndex<K, V>;
        const change = sqIndex.determineChange(key, record, undefined);
        if (change !== 'unchanged') {
          sqIndex.remove(key, record);
          deltas.push({
            queryHash: hash,
            query: entry.query,
            delta: change,
            isFTS: false,
          });
        }
      }
    }

    return deltas;
  }

  /**
   * Get results for a query.
   * Returns keys for standing query or ranked results for FTS.
   *
   * @param query - Query to get results for
   * @returns Results array or undefined if not registered
   */
  getResults(query: Query): K[] | RankedResult<K>[] | undefined {
    const hash = this.hashQuery(query);
    const entry = this.indexes.get(hash);
    if (!entry) return undefined;

    if (entry.isFTS) {
      return (entry.index as LiveFTSIndex<K, V>).getResults();
    } else {
      return Array.from((entry.index as StandingQueryIndex<K, V>).getResults());
    }
  }

  /**
   * Get all registered queries.
   *
   * @returns Array of registered queries
   */
  getRegisteredQueries(): Query[] {
    return Array.from(this.indexes.values()).map((e) => e.query);
  }

  /**
   * Get all query hashes.
   *
   * @returns Array of query hashes
   */
  getQueryHashes(): string[] {
    return Array.from(this.indexes.keys());
  }

  /**
   * Get statistics about the registry.
   *
   * @returns Registry statistics
   */
  getStats(): UnifiedLiveQueryRegistryStats {
    let ftsCount = 0;
    let standingCount = 0;
    let totalResults = 0;

    for (const entry of this.indexes.values()) {
      if (entry.isFTS) {
        ftsCount++;
        totalResults += (entry.index as LiveFTSIndex<K, V>).getResultCount();
      } else {
        standingCount++;
        totalResults += (entry.index as StandingQueryIndex<K, V>).getResultCount();
      }
    }

    return {
      indexCount: this.indexes.size,
      ftsIndexCount: ftsCount,
      standingIndexCount: standingCount,
      totalRefCount: Array.from(this.refCounts.values()).reduce((a, b) => a + b, 0),
      totalResults,
    };
  }

  /**
   * Clear all indexes.
   */
  clear(): void {
    for (const entry of this.indexes.values()) {
      entry.index.clear();
    }
    this.indexes.clear();
    this.refCounts.clear();
  }

  /**
   * Get number of registered indexes.
   */
  get size(): number {
    return this.indexes.size;
  }

  /**
   * Compute hash for a query.
   */
  hashQuery(query: Query): string {
    return JSON.stringify(query);
  }

  /**
   * Check if a query contains any FTS predicates.
   * Recursively checks logical query children.
   *
   * @param query - Query to check
   * @returns true if query contains FTS predicates
   */
  containsFTSPredicate(query: Query): boolean {
    if (isFTSQuery(query)) {
      return true;
    }

    if (isLogicalQuery(query)) {
      const logicalQuery = query as LogicalQueryNode;
      if (logicalQuery.children) {
        return logicalQuery.children.some((child) => this.containsFTSPredicate(child));
      }
      if (logicalQuery.child) {
        return this.containsFTSPredicate(logicalQuery.child);
      }
    }

    return false;
  }

  /**
   * Extract FTS info from query for creating LiveFTSIndex.
   * Currently only handles simple match queries at the root level.
   * For complex hybrid queries, returns null (falls back to StandingQueryIndex).
   *
   * @param query - Query to extract FTS info from
   * @returns FTS info or null if not a simple FTS query
   */
  private extractFTSInfo(
    query: Query
  ): { field: string; query: string; minScore?: number; maxResults?: number } | null {
    // Only handle simple match queries at root level for now
    // Hybrid queries (FTS + filter) will need QueryExecutor
    if (isMatchQuery(query)) {
      const matchQuery = query as MatchQueryNode;
      return {
        field: matchQuery.attribute,
        query: matchQuery.query,
        minScore: matchQuery.options?.minScore,
      };
    }

    // For logical queries with FTS, we'd need the QueryExecutor to handle fusion
    // Fall back to StandingQueryIndex for now (it can't evaluate FTS though)
    return null;
  }
}
