/**
 * LimitResultSet Implementation
 *
 * Applies offset/limit to source ResultSet.
 * Implements early termination for efficiency.
 *
 * NOTE: The offset parameter is intentionally retained in this internal component
 * even though cursor-based pagination replaced offset in the query API.
 * This class is used internally by:
 * - EventJournalService (SQL queries require numeric offset)
 * - Index result set operations where offset is computed internally
 * - Unit tests for result set behavior
 *
 * The query API (QueryOptions, QueryHandle) uses cursor-based pagination via QueryCursor.
 *
 * @module query/resultset/LimitResultSet
 */

import type { ResultSet } from './ResultSet';

/**
 * Applies offset/limit to source ResultSet.
 * Implements early termination for efficiency.
 *
 * K = record key type
 */
export class LimitResultSet<K> implements ResultSet<K> {
  /** Cached materialized results */
  private cached: K[] | null = null;

  /**
   * Create a LimitResultSet.
   *
   * @param source - Source result set
   * @param offset - Number of results to skip (default: 0)
   * @param limit - Maximum number of results (default: Infinity)
   */
  constructor(
    private readonly source: ResultSet<K>,
    private readonly offset: number = 0,
    private readonly limit: number = Infinity
  ) {}

  /**
   * Lazy iteration with offset/limit and early termination.
   */
  *[Symbol.iterator](): Generator<K> {
    // Use cached results if available
    if (this.cached) {
      yield* this.cached;
      return;
    }

    // If no offset and no limit, just pass through
    if (this.offset === 0 && this.limit === Infinity) {
      yield* this.source;
      return;
    }

    let skipped = 0;
    let returned = 0;

    for (const key of this.source) {
      // Skip offset
      if (skipped < this.offset) {
        skipped++;
        continue;
      }

      // Early termination when limit reached
      if (returned >= this.limit) {
        break;
      }

      yield key;
      returned++;
    }
  }

  /**
   * Retrieval cost: source cost (limit doesn't change retrieval cost).
   */
  getRetrievalCost(): number {
    return this.source.getRetrievalCost();
  }

  /**
   * Merge cost: min(source size, offset + limit).
   */
  getMergeCost(): number {
    const sourceCost = this.source.getMergeCost();
    if (this.limit === Infinity) {
      return Math.max(0, sourceCost - this.offset);
    }
    return Math.min(sourceCost, this.offset + this.limit);
  }

  /**
   * Check if key is in result (with offset/limit constraints).
   * This is expensive as it requires iteration to determine position.
   */
  contains(key: K): boolean {
    // First check if key is in source at all
    if (!this.source.contains(key)) {
      return false;
    }

    // Need to materialize to check if key is within offset/limit
    return this.toArray().includes(key);
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
   * Check if empty.
   */
  isEmpty(): boolean {
    // If cached, use cached value
    if (this.cached) {
      return this.cached.length === 0;
    }

    // If limit is 0, always empty
    if (this.limit === 0) {
      return true;
    }

    // Try to get first element
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

  /**
   * Get the offset value.
   */
  getOffset(): number {
    return this.offset;
  }

  /**
   * Get the limit value.
   */
  getLimit(): number {
    return this.limit;
  }
}
