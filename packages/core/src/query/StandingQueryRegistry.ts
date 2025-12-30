/**
 * StandingQueryRegistry Implementation
 *
 * Registry for managing StandingQueryIndexes.
 * Used by Live Query system to maintain pre-computed results.
 *
 * Features:
 * - Reference counting for shared indexes
 * - Automatic cleanup when all subscribers unsubscribe
 * - Efficient update propagation to all indexes
 *
 * @module query/StandingQueryRegistry
 */

import {
  StandingQueryIndex,
  type StandingQueryChange,
  type StandingQueryIndexOptions,
} from './indexes/StandingQueryIndex';
import type { Query } from './QueryTypes';

/**
 * Options for creating a StandingQueryRegistry.
 */
export interface StandingQueryRegistryOptions<K, V> {
  /** Function to get record by key */
  getRecord: (key: K) => V | undefined;
  /** Function to get all entries for building index */
  getAllEntries: () => Iterable<[K, V]>;
}

/**
 * Statistics about the StandingQueryRegistry.
 */
export interface StandingQueryRegistryStats {
  /** Number of registered indexes */
  indexCount: number;
  /** Total reference count across all indexes */
  totalRefCount: number;
  /** Total number of results across all indexes */
  totalResults: number;
}

/**
 * Registry for managing StandingQueryIndexes.
 * Provides reference counting and lifecycle management.
 *
 * K = record key type, V = record value type
 */
export class StandingQueryRegistry<K, V> {
  /** Map from query hash to StandingQueryIndex */
  private indexes: Map<string, StandingQueryIndex<K, V>> = new Map();

  /** Reference count for each query (multiple subscriptions can use same index) */
  private refCounts: Map<string, number> = new Map();

  /** Record accessor */
  private readonly getRecord: (key: K) => V | undefined;

  /** All entries accessor (for building index) */
  private readonly getAllEntries: () => Iterable<[K, V]>;

  constructor(options: StandingQueryRegistryOptions<K, V>) {
    this.getRecord = options.getRecord;
    this.getAllEntries = options.getAllEntries;
  }

  /**
   * Register a standing query.
   * Creates new index or returns existing if query already registered.
   * Increments reference count.
   *
   * @param query - Query to register
   * @returns StandingQueryIndex for the query
   */
  register(query: Query): StandingQueryIndex<K, V> {
    const hash = this.hashQuery(query);

    let index = this.indexes.get(hash);
    if (index) {
      // Increment reference count
      this.refCounts.set(hash, (this.refCounts.get(hash) || 0) + 1);
      return index;
    }

    // Create new index
    const options: StandingQueryIndexOptions<K, V> = {
      query,
      getRecord: this.getRecord,
    };
    index = new StandingQueryIndex(options);

    // Build from existing data
    index.buildFromData(this.getAllEntries());

    this.indexes.set(hash, index);
    this.refCounts.set(hash, 1);

    return index;
  }

  /**
   * Unregister a standing query.
   * Decrements reference count. Only removes when refcount reaches 0.
   *
   * @param query - Query to unregister
   * @returns true if index was removed, false if still has references
   */
  unregister(query: Query): boolean {
    const hash = this.hashQuery(query);
    const refCount = this.refCounts.get(hash) || 0;

    if (refCount <= 1) {
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
   * @returns StandingQueryIndex or undefined if not registered
   */
  getIndex(query: Query): StandingQueryIndex<K, V> | undefined {
    const hash = this.hashQuery(query);
    return this.indexes.get(hash);
  }

  /**
   * Get index by hash directly.
   *
   * @param hash - Query hash
   * @returns StandingQueryIndex or undefined if not registered
   */
  getIndexByHash(hash: string): StandingQueryIndex<K, V> | undefined {
    return this.indexes.get(hash);
  }

  /**
   * Check if query has a standing index.
   *
   * @param query - Query to check
   * @returns true if index exists
   */
  hasIndex(query: Query): boolean {
    const hash = this.hashQuery(query);
    return this.indexes.has(hash);
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
   * Returns map of query hash to change type for affected queries.
   *
   * @param key - Record key
   * @param record - New record value
   * @returns Map of query hash to change type
   */
  onRecordAdded(key: K, record: V): Map<string, StandingQueryChange> {
    const changes = new Map<string, StandingQueryChange>();

    for (const [hash, index] of this.indexes) {
      const change = index.determineChange(key, undefined, record);
      if (change !== 'unchanged') {
        index.add(key, record);
        changes.set(hash, change);
      }
    }

    return changes;
  }

  /**
   * Notify all indexes of record update.
   * Returns map of query hash to change type for affected queries.
   *
   * @param key - Record key
   * @param oldRecord - Previous record value
   * @param newRecord - New record value
   * @returns Map of query hash to change type
   */
  onRecordUpdated(
    key: K,
    oldRecord: V,
    newRecord: V
  ): Map<string, StandingQueryChange> {
    const changes = new Map<string, StandingQueryChange>();

    for (const [hash, index] of this.indexes) {
      const change = index.determineChange(key, oldRecord, newRecord);
      if (change !== 'unchanged') {
        index.update(key, oldRecord, newRecord);
        changes.set(hash, change);
      }
    }

    return changes;
  }

  /**
   * Notify all indexes of record removal.
   * Returns map of query hash to change type for affected queries.
   *
   * @param key - Record key
   * @param record - Removed record value
   * @returns Map of query hash to change type
   */
  onRecordRemoved(key: K, record: V): Map<string, StandingQueryChange> {
    const changes = new Map<string, StandingQueryChange>();

    for (const [hash, index] of this.indexes) {
      const change = index.determineChange(key, record, undefined);
      if (change !== 'unchanged') {
        index.remove(key, record);
        changes.set(hash, change);
      }
    }

    return changes;
  }

  /**
   * Get all registered queries.
   *
   * @returns Array of registered queries
   */
  getRegisteredQueries(): Query[] {
    return Array.from(this.indexes.values()).map((idx) => idx.getQuery());
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
  getStats(): StandingQueryRegistryStats {
    return {
      indexCount: this.indexes.size,
      totalRefCount: Array.from(this.refCounts.values()).reduce(
        (a, b) => a + b,
        0
      ),
      totalResults: Array.from(this.indexes.values()).reduce(
        (sum, idx) => sum + idx.getResultCount(),
        0
      ),
    };
  }

  /**
   * Clear all indexes.
   */
  clear(): void {
    for (const index of this.indexes.values()) {
      index.clear();
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
   * Used as key in indexes map.
   */
  hashQuery(query: Query): string {
    return JSON.stringify(query);
  }
}
