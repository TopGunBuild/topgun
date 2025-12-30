/**
 * IntersectionResultSet Implementation
 *
 * Intersection of multiple result sets (AND logic).
 * Implements CQEngine "smallest first" strategy:
 * iterate the smallest set, check membership in others.
 *
 * @module query/resultset/IntersectionResultSet
 */

import type { ResultSet } from './ResultSet';

/**
 * Intersection of multiple result sets (AND logic).
 * CQEngine strategy: iterate smallest set, check membership in others.
 *
 * K = record key type
 */
export class IntersectionResultSet<K> implements ResultSet<K> {
  /** Cached materialized results */
  private cached: K[] | null = null;

  /** Result sets sorted by merge cost */
  private sortedResultSets: ResultSet<K>[];

  /**
   * Create an IntersectionResultSet.
   *
   * @param resultSets - Result sets to intersect
   */
  constructor(resultSets: ResultSet<K>[]) {
    // Sort by merge cost (ascending) - iterate smallest first
    this.sortedResultSets = [...resultSets].sort(
      (a, b) => a.getMergeCost() - b.getMergeCost()
    );
  }

  /**
   * Lazy iteration over intersection.
   * Iterates smallest set, yields only keys present in all sets.
   */
  *[Symbol.iterator](): Generator<K> {
    // Use cached results if available
    if (this.cached) {
      yield* this.cached;
      return;
    }

    // Empty intersection if no result sets
    if (this.sortedResultSets.length === 0) {
      return;
    }

    // Get smallest set (first after sort)
    const [smallest, ...rest] = this.sortedResultSets;

    // Iterate smallest, check membership in all others
    for (const key of smallest) {
      if (rest.every((rs) => rs.contains(key))) {
        yield key;
      }
    }
  }

  /**
   * Retrieval cost is minimum of all (we only iterate smallest).
   */
  getRetrievalCost(): number {
    if (this.sortedResultSets.length === 0) {
      return 0;
    }
    return Math.min(...this.sortedResultSets.map((rs) => rs.getRetrievalCost()));
  }

  /**
   * Merge cost is estimated as smallest set size (upper bound).
   */
  getMergeCost(): number {
    if (this.sortedResultSets.length === 0) {
      return 0;
    }
    // After sorting, first has smallest merge cost
    return this.sortedResultSets[0].getMergeCost();
  }

  /**
   * Check if key is in all result sets.
   */
  contains(key: K): boolean {
    return this.sortedResultSets.every((rs) => rs.contains(key));
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
   * Check if empty (tries to avoid full materialization).
   */
  isEmpty(): boolean {
    // If cached, use cached value
    if (this.cached) {
      return this.cached.length === 0;
    }

    // If any source is empty, intersection is empty
    if (this.sortedResultSets.some((rs) => rs.isEmpty())) {
      return true;
    }

    // Check by trying to get first element
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
