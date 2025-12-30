/**
 * Lazy Indexes exports (Phase 9.01)
 *
 * Lazy indexes defer actual index construction until first query.
 * This provides fast application startup and memory efficiency.
 */

export type {
  LazyIndex,
  LazyIndexOptions,
} from './types';
export { isLazyIndex } from './types';
export { LazyHashIndex } from './LazyHashIndex';
export { LazyNavigableIndex } from './LazyNavigableIndex';
export { LazyInvertedIndex } from './LazyInvertedIndex';

// Re-export IndexBuildProgressCallback from adaptive for convenience
export type { IndexBuildProgressCallback } from '../../adaptive/types';
