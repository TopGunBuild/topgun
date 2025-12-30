/**
 * ResultSet Interface
 *
 * Lazy result set from index queries.
 * Inspired by CQEngine ResultSet<O>.
 *
 * @module query/resultset/ResultSet
 */

/**
 * Lazy result set from index query.
 * K = record key type
 */
export interface ResultSet<K> {
  /**
   * Iterate over matching keys.
   * Lazy evaluation where possible.
   */
  [Symbol.iterator](): Iterator<K>;

  /**
   * Cost of retrieving these results.
   * Used by QueryOptimizer for index selection.
   */
  getRetrievalCost(): number;

  /**
   * Estimated cost of merging/processing these results.
   * Usually based on result count.
   */
  getMergeCost(): number;

  /**
   * Check if result set contains key.
   */
  contains(key: K): boolean;

  /**
   * Get result count.
   * May require iteration for lazy result sets.
   */
  size(): number;

  /**
   * Materialize to array.
   */
  toArray(): K[];

  /**
   * Check if empty.
   */
  isEmpty(): boolean;
}
