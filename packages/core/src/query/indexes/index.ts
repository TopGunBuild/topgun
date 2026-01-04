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

// Compound Index (Phase 9.03)
export {
  CompoundIndex,
  isCompoundIndex,
  type CompoundQuery,
  type CompoundIndexOptions,
  type CompoundIndexStats,
} from './CompoundIndex';

// Lazy indexes (Phase 9.01)
// Note: IndexBuildProgressCallback is exported from ./adaptive
export {
  LazyHashIndex,
  LazyNavigableIndex,
  LazyInvertedIndex,
  isLazyIndex,
  type LazyIndex,
  type LazyIndexOptions,
} from './lazy';

// Live Query Indexes (Phase 12)
export {
  type ILiveQueryIndex,
  type LiveQueryDelta,
  type RankedResult,
  type LiveFTSIndexOptions,
} from './ILiveQueryIndex';
export { LiveFTSIndex } from './LiveFTSIndex';
