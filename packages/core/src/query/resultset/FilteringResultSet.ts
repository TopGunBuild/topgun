/**
 * FilteringResultSet Implementation
 *
 * Filters a source result set with a predicate.
 * Used when an index is available for part of a query,
 * but additional filtering is needed.
 *
 * @module query/resultset/FilteringResultSet
 */

import type { ResultSet } from './ResultSet';

/**
 * Predicate function for filtering records.
 */
export type PredicateFn<V> = (record: V) => boolean;

/**
 * Filters a source result set with a predicate.
 *
 * K = record key type, V = record value type
 */
export class FilteringResultSet<K, V> implements ResultSet<K> {
  /** Cached materialized results */
  private cached: K[] | null = null;

  /**
   * Create a FilteringResultSet.
   *
   * @param source - Source result set to filter
   * @param getRecord - Function to get record by key
   * @param predicate - Predicate function to filter records
   */
  constructor(
    private readonly source: ResultSet<K>,
    private readonly getRecord: (key: K) => V | undefined,
    private readonly predicate: PredicateFn<V>
  ) {}

  /**
   * Lazy iteration with filtering.
   */
  *[Symbol.iterator](): Generator<K> {
    // Use cached results if available
    if (this.cached) {
      yield* this.cached;
      return;
    }

    for (const key of this.source) {
      const record = this.getRecord(key);
      if (record !== undefined && this.predicate(record)) {
        yield key;
      }
    }
  }

  /**
   * Retrieval cost: source cost + filter overhead.
   */
  getRetrievalCost(): number {
    return this.source.getRetrievalCost() + 10;
  }

  /**
   * Merge cost: estimate half of source (pessimistic).
   */
  getMergeCost(): number {
    return Math.max(1, Math.ceil(this.source.getMergeCost() / 2));
  }

  /**
   * Check if key is in source and passes predicate.
   */
  contains(key: K): boolean {
    if (!this.source.contains(key)) {
      return false;
    }
    const record = this.getRecord(key);
    return record !== undefined && this.predicate(record);
  }

  /**
   * Get size by materializing results.
   */
  size(): number {
    return this.toArray().length;
  }

  /**
   * Materialize to array with caching.
   */
  toArray(): K[] {
    if (!this.cached) {
      this.cached = [...this];
    }
    return this.cached;
  }

  /**
   * Check if empty (tries to find at least one match).
   */
  isEmpty(): boolean {
    // If cached, use cached value
    if (this.cached) {
      return this.cached.length === 0;
    }

    // Try to find at least one matching element
    for (const _ of this) {
      return false;
    }
    return true;
  }

  /**
   * Check if results have been materialized.
   */
  isMaterialized(): boolean {
    return this.cached !== null;
  }
}
