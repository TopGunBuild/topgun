/**
 * Indexes exports
 */

export type { Index, IndexQuery, IndexStats } from './types';
export { HashIndex } from './HashIndex';
export { NavigableIndex } from './NavigableIndex';
export {
  QuantizedNavigableIndex,
  Quantizers,
  type Quantizer,
} from './QuantizedNavigableIndex';
export { FallbackIndex, createPredicateMatcher } from './FallbackIndex';
export {
  StandingQueryIndex,
  type StandingQueryIndexOptions,
  type StandingQueryChange,
} from './StandingQueryIndex';
export { InvertedIndex, type InvertedIndexStats } from './InvertedIndex';

// Compound Index
export {
  CompoundIndex,
  isCompoundIndex,
  type CompoundQuery,
  type CompoundIndexOptions,
  type CompoundIndexStats,
} from './CompoundIndex';

// Lazy indexes
// Note: IndexBuildProgressCallback is exported from ./adaptive
export {
  LazyHashIndex,
  LazyNavigableIndex,
  LazyInvertedIndex,
  isLazyIndex,
  type LazyIndex,
  type LazyIndexOptions,
} from './lazy';

// Live Query Indexes
export {
  type ILiveQueryIndex,
  type LiveQueryDelta,
  type RankedResult,
  type LiveFTSIndexOptions,
} from './ILiveQueryIndex';
export { LiveFTSIndex } from './LiveFTSIndex';
