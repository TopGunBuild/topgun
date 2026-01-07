/**
 * Search Module
 *
 * Server-side full-text search functionality.
 *
 * @module search
 */

export { SearchCoordinator } from './SearchCoordinator';
export type { SearchConfig, ServerSearchResult } from './SearchCoordinator';

// Phase 14: Distributed Search
export { ClusterSearchCoordinator } from './ClusterSearchCoordinator';
export type {
  ClusterSearchConfig,
  DistributedSearchOptions,
  DistributedSearchResult,
} from './ClusterSearchCoordinator';
