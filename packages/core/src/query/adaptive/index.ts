/**
 * Adaptive Indexing System
 *
 * Exports for query pattern tracking, index suggestions,
 * and automatic index creation.
 *
 * @module query/adaptive
 */

// Types
export * from './types';

// Core components
export { QueryPatternTracker } from './QueryPatternTracker';
export type { QueryPatternTrackerOptions } from './QueryPatternTracker';

export { IndexAdvisor } from './IndexAdvisor';

export { AutoIndexManager } from './AutoIndexManager';
export type { IndexableMap } from './AutoIndexManager';

export { DefaultIndexingStrategy } from './DefaultIndexingStrategy';
export type {
  FieldInfo,
  FieldIndexRecommendation,
  DefaultIndexableMap,
} from './DefaultIndexingStrategy';
