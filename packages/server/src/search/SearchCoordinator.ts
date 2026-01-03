/**
 * SearchCoordinator - Server-side Full-Text Search Handler
 *
 * Manages FullTextIndex instances per map and handles search requests.
 * Part of Phase 11.1a: Server-side BM25 Search.
 *
 * @module search/SearchCoordinator
 */

import {
  FullTextIndex,
  type FullTextIndexConfig,
  type FTSSearchOptions as SearchOptions,
  type FTSSearchResult as SearchResult,
  type SearchRespPayload,
} from '@topgunbuild/core';
import { logger } from '../utils/logger';

/**
 * Result item returned from server search.
 */
export interface ServerSearchResult {
  key: string;
  value: unknown;
  score: number;
  matchedTerms: string[];
}

/**
 * Configuration for enabling search on a map.
 */
export interface SearchConfig extends FullTextIndexConfig {
  // Additional server-specific options can be added here in the future
}

/**
 * SearchCoordinator manages full-text search indexes for the server.
 *
 * Responsibilities:
 * - Maintain FullTextIndex per enabled map
 * - Execute one-shot search queries
 * - Update indexes when data changes
 *
 * @example
 * ```typescript
 * const searchCoordinator = new SearchCoordinator();
 *
 * // Enable FTS for a map
 * searchCoordinator.enableSearch('articles', {
 *   fields: ['title', 'body'],
 *   tokenizer: { minLength: 2 },
 *   bm25: { k1: 1.2, b: 0.75 }
 * });
 *
 * // Search
 * const results = searchCoordinator.search('articles', 'machine learning', {
 *   limit: 20,
 *   boost: { title: 2.0 }
 * });
 * ```
 */
export class SearchCoordinator {
  /** Map name → FullTextIndex */
  private readonly indexes: Map<string, FullTextIndex> = new Map();

  /** Map name → FullTextIndexConfig (for reference) */
  private readonly configs: Map<string, SearchConfig> = new Map();

  /** Callback to get document value by key (injected by ServerCoordinator) */
  private getDocumentValue?: (mapName: string, key: string) => unknown | undefined;

  constructor() {
    logger.debug('SearchCoordinator initialized');
  }

  /**
   * Set the callback for retrieving document values.
   * Called by ServerCoordinator during initialization.
   */
  setDocumentValueGetter(getter: (mapName: string, key: string) => unknown | undefined): void {
    this.getDocumentValue = getter;
  }

  /**
   * Enable full-text search for a map.
   *
   * @param mapName - Name of the map to enable FTS for
   * @param config - FTS configuration (fields, tokenizer, bm25 options)
   */
  enableSearch(mapName: string, config: SearchConfig): void {
    if (this.indexes.has(mapName)) {
      logger.warn({ mapName }, 'FTS already enabled for map, replacing index');
      this.indexes.delete(mapName);
    }

    const index = new FullTextIndex(config);
    this.indexes.set(mapName, index);
    this.configs.set(mapName, config);

    logger.info({ mapName, fields: config.fields }, 'FTS enabled for map');
  }

  /**
   * Disable full-text search for a map.
   *
   * @param mapName - Name of the map to disable FTS for
   */
  disableSearch(mapName: string): void {
    if (!this.indexes.has(mapName)) {
      logger.warn({ mapName }, 'FTS not enabled for map, nothing to disable');
      return;
    }

    this.indexes.delete(mapName);
    this.configs.delete(mapName);

    logger.info({ mapName }, 'FTS disabled for map');
  }

  /**
   * Check if FTS is enabled for a map.
   */
  isSearchEnabled(mapName: string): boolean {
    return this.indexes.has(mapName);
  }

  /**
   * Get enabled map names.
   */
  getEnabledMaps(): string[] {
    return Array.from(this.indexes.keys());
  }

  /**
   * Execute a one-shot search query.
   *
   * @param mapName - Name of the map to search
   * @param query - Search query text
   * @param options - Search options (limit, minScore, boost)
   * @returns Search response payload
   */
  search(
    mapName: string,
    query: string,
    options?: SearchOptions
  ): SearchRespPayload {
    const index = this.indexes.get(mapName);

    if (!index) {
      logger.warn({ mapName }, 'Search requested for map without FTS enabled');
      return {
        requestId: '',
        results: [],
        totalCount: 0,
        error: `Full-text search not enabled for map: ${mapName}`,
      };
    }

    try {
      // Execute search
      const searchResults = index.search(query, options);

      // Map results to include document values
      const results: ServerSearchResult[] = searchResults.map((result) => {
        // Get the actual document value if getter is available
        const value = this.getDocumentValue
          ? this.getDocumentValue(mapName, result.docId)
          : undefined;

        return {
          key: result.docId,
          value,
          score: result.score,
          matchedTerms: result.matchedTerms || [],
        };
      });

      logger.debug(
        { mapName, query, resultCount: results.length },
        'Search executed'
      );

      return {
        requestId: '',
        results,
        totalCount: searchResults.length,
      };
    } catch (err) {
      logger.error({ mapName, query, err }, 'Search failed');
      return {
        requestId: '',
        results: [],
        totalCount: 0,
        error: `Search failed: ${(err as Error).message}`,
      };
    }
  }

  /**
   * Handle document set/update.
   * Called by ServerCoordinator when data changes.
   *
   * @param mapName - Name of the map
   * @param key - Document key
   * @param value - Document value
   */
  onDataChange(
    mapName: string,
    key: string,
    value: Record<string, unknown> | null | undefined,
    changeType: 'add' | 'update' | 'remove'
  ): void {
    const index = this.indexes.get(mapName);
    if (!index) {
      return; // FTS not enabled for this map
    }

    if (changeType === 'remove' || value === null || value === undefined) {
      index.onRemove(key);
    } else {
      index.onSet(key, value);
    }
  }

  /**
   * Build index from existing map entries.
   * Called when FTS is enabled for a map that already has data.
   *
   * @param mapName - Name of the map
   * @param entries - Iterator of [key, value] tuples
   */
  buildIndexFromEntries(
    mapName: string,
    entries: Iterable<[string, Record<string, unknown> | null]>
  ): void {
    const index = this.indexes.get(mapName);
    if (!index) {
      logger.warn({ mapName }, 'Cannot build index: FTS not enabled for map');
      return;
    }

    let count = 0;
    for (const [key, value] of entries) {
      if (value !== null) {
        index.onSet(key, value);
        count++;
      }
    }

    logger.info({ mapName, documentCount: count }, 'Index built from entries');
  }

  /**
   * Get index statistics for monitoring.
   */
  getIndexStats(mapName: string): { documentCount: number; fields: string[] } | null {
    const index = this.indexes.get(mapName);
    const config = this.configs.get(mapName);

    if (!index || !config) {
      return null;
    }

    return {
      documentCount: index.getSize(),
      fields: config.fields,
    };
  }

  /**
   * Clear all indexes (for testing or shutdown).
   */
  clear(): void {
    for (const index of this.indexes.values()) {
      index.clear();
    }
    this.indexes.clear();
    this.configs.clear();
    logger.debug('SearchCoordinator cleared');
  }
}
