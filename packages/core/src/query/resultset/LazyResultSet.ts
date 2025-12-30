/**
 * LazyResultSet Implementation
 *
 * Lazily evaluated result set for range queries.
 * Used when materializing all results upfront would be expensive.
 *
 * @module query/resultset/LazyResultSet
 */

import type { ResultSet } from './ResultSet';

/**
 * Factory function type that creates a generator for lazy iteration.
 */
export type IteratorFactory<K> = () => Generator<K>;

/**
 * Lazily evaluated result set.
 * Used for range queries where materializing all results upfront is expensive.
 *
 * K = record key type
 */
export class LazyResultSet<K> implements ResultSet<K> {
  /** Cached materialized results */
  private cached: K[] | null = null;

  /**
   * Create a LazyResultSet.
   *
   * @param iteratorFactory - Factory that creates a fresh generator each time
   * @param retrievalCost - Cost of retrieving results from the index
   * @param estimatedSize - Estimated result count for merge cost calculation
   */
  constructor(
    private readonly iteratorFactory: IteratorFactory<K>,
    private readonly retrievalCost: number,
    private readonly estimatedSize: number
  ) {}

  *[Symbol.iterator](): Generator<K> {
    if (this.cached) {
      yield* this.cached;
      return;
    }

    yield* this.iteratorFactory();
  }

  getRetrievalCost(): number {
    return this.retrievalCost;
  }

  getMergeCost(): number {
    // Use actual size if cached, otherwise use estimated size
    return this.cached?.length ?? this.estimatedSize;
  }

  contains(key: K): boolean {
    // Must materialize to check containment
    return this.toArray().includes(key);
  }

  size(): number {
    return this.toArray().length;
  }

  toArray(): K[] {
    if (!this.cached) {
      this.cached = [...this.iteratorFactory()];
    }
    return this.cached;
  }

  isEmpty(): boolean {
    // If already cached, use the cached value
    if (this.cached) {
      return this.cached.length === 0;
    }

    // Try to avoid full materialization by checking just the first element
    const iter = this.iteratorFactory();
    const first = iter.next();
    return first.done === true;
  }

  /**
   * Check if the result set has been materialized.
   * Useful for testing lazy evaluation behavior.
   */
  isMaterialized(): boolean {
    return this.cached !== null;
  }

  /**
   * Force materialization of the result set.
   * Returns the cached array.
   */
  materialize(): K[] {
    return this.toArray();
  }

  /**
   * Get the estimated size before materialization.
   */
  getEstimatedSize(): number {
    return this.estimatedSize;
  }
}
