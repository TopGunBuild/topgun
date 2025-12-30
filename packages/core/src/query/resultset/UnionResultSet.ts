/**
 * UnionResultSet Implementation
 *
 * Union of multiple result sets (OR logic).
 * Deduplicates results using a Set during iteration.
 *
 * @module query/resultset/UnionResultSet
 */

import type { ResultSet } from './ResultSet';

/**
 * Union of multiple result sets (OR logic).
 * Deduplicates results.
 *
 * K = record key type
 */
export class UnionResultSet<K> implements ResultSet<K> {
  /** Cached materialized results */
  private cached: K[] | null = null;

  /**
   * Create a UnionResultSet.
   *
   * @param resultSets - Result sets to union
   */
  constructor(private readonly resultSets: ResultSet<K>[]) {}

  /**
   * Lazy iteration over union with deduplication.
   */
  *[Symbol.iterator](): Generator<K> {
    // Use cached results if available
    if (this.cached) {
      yield* this.cached;
      return;
    }

    // Track seen keys to avoid duplicates
    const seen = new Set<K>();

    for (const rs of this.resultSets) {
      for (const key of rs) {
        if (!seen.has(key)) {
          seen.add(key);
          yield key;
        }
      }
    }
  }

  /**
   * Retrieval cost is sum of all costs.
   */
  getRetrievalCost(): number {
    return this.resultSets.reduce((sum, rs) => {
      const cost = rs.getRetrievalCost();
      // Avoid overflow
      if (cost === Number.MAX_SAFE_INTEGER || sum === Number.MAX_SAFE_INTEGER) {
        return Number.MAX_SAFE_INTEGER;
      }
      return Math.min(sum + cost, Number.MAX_SAFE_INTEGER);
    }, 0);
  }

  /**
   * Merge cost upper bound: sum of all sizes.
   */
  getMergeCost(): number {
    return this.resultSets.reduce((sum, rs) => sum + rs.getMergeCost(), 0);
  }

  /**
   * Check if key is in any result set.
   */
  contains(key: K): boolean {
    return this.resultSets.some((rs) => rs.contains(key));
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
   * Check if empty (all sources must be empty).
   */
  isEmpty(): boolean {
    // If cached, use cached value
    if (this.cached) {
      return this.cached.length === 0;
    }

    // Union is empty only if all sources are empty
    return this.resultSets.every((rs) => rs.isEmpty());
  }

  /**
   * Check if results have been materialized.
   */
  isMaterialized(): boolean {
    return this.cached !== null;
  }
}
