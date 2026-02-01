/**
 * Lazy Index Types
 *
 * Types and interfaces for lazy index building.
 * Lazy indexes defer actual index construction until first query.
 *
 * @module query/indexes/lazy/types
 */

import type { Index } from '../types';
import type { IndexBuildProgressCallback } from '../../adaptive/types';

/**
 * Extended interface for lazy indexes.
 * Adds lazy-specific properties and methods.
 */
export interface LazyIndex<K, V, A = unknown> extends Index<K, V, A> {
  /**
   * Whether the index has been materialized (built).
   */
  readonly isBuilt: boolean;

  /**
   * Number of pending records awaiting materialization.
   */
  readonly pendingCount: number;

  /**
   * Force materialization of the index.
   * Called automatically on first query, but can be called manually.
   *
   * @param progressCallback - Optional progress callback
   */
  materialize(progressCallback?: IndexBuildProgressCallback): void;

  /**
   * Check if this is a lazy index wrapper.
   */
  readonly isLazy: true;
}

/**
 * Options for lazy index creation.
 */
export interface LazyIndexOptions {
  /**
   * Progress callback for index building.
   */
  onProgress?: IndexBuildProgressCallback;

  /**
   * Batch size for progress reporting.
   * Default: 1000
   */
  progressBatchSize?: number;
}

/**
 * Check if an index is a lazy index.
 */
export function isLazyIndex<K, V, A>(
  index: Index<K, V, A>
): index is LazyIndex<K, V, A> {
  return 'isLazy' in index && (index as LazyIndex<K, V, A>).isLazy === true;
}
