/**
 * SearchCoordinator - Server-side Full-Text Search Handler
 *
 * Manages FullTextIndex instances per map and handles search requests.
 * Part of Phase 11.1a: Server-side BM25 Search.
 * Phase 11.1b: Live Search Subscriptions with delta updates.
 *
 * @module search/SearchCoordinator
 */

import {
  FullTextIndex,
  type FullTextIndexConfig,
  type FTSSearchOptions as SearchOptions,
  type FTSSearchResult as SearchResult,
  type SearchRespPayload,
  type SearchUpdateType,
  type SearchOptions as SchemaSearchOptions,
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
 * Cached result for a document in a subscription.
 */
interface CachedResult {
  score: number;
  matchedTerms: string[];
}

/**
 * Represents a live search subscription.
 */
interface SearchSubscription {
  /** Unique subscription ID */
  id: string;
  /** ID of the subscribed client */
  clientId: string;
  /** Name of the map being searched */
  mapName: string;
  /** Original query string */
  query: string;
  /** Tokenized query terms for fast comparison */
  queryTerms: string[];
  /** Search options (limit, minScore, boost) */
  options: SchemaSearchOptions;
  /** Cache of current results for delta computation */
  currentResults: Map<string, CachedResult>;
}

/**
 * Callback type for sending updates to clients.
 */
export type SendUpdateCallback = (
  clientId: string,
  subscriptionId: string,
  key: string,
  value: unknown,
  score: number,
  matchedTerms: string[],
  type: SearchUpdateType
) => void;

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

  // ============================================
  // Phase 11.1b: Live Search Subscription tracking
  // ============================================

  /** Subscription ID → SearchSubscription */
  private readonly subscriptions: Map<string, SearchSubscription> = new Map();

  /** Map name → Set of subscription IDs */
  private readonly subscriptionsByMap: Map<string, Set<string>> = new Map();

  /** Client ID → Set of subscription IDs */
  private readonly subscriptionsByClient: Map<string, Set<string>> = new Map();

  /** Callback for sending updates to clients */
  private sendUpdate?: SendUpdateCallback;

  constructor() {
    logger.debug('SearchCoordinator initialized');
  }

  /**
   * Set the callback for sending updates to clients.
   * Called by ServerCoordinator during initialization.
   */
  setSendUpdateCallback(callback: SendUpdateCallback): void {
    this.sendUpdate = callback;
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

    // Phase 11.1b: Notify subscribers of potential changes
    this.notifySubscribers(mapName, key, value ?? null, changeType);
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
    // Phase 11.1b: Clear subscriptions
    this.subscriptions.clear();
    this.subscriptionsByMap.clear();
    this.subscriptionsByClient.clear();
    logger.debug('SearchCoordinator cleared');
  }

  // ============================================
  // Phase 11.1b: Live Search Subscription Methods
  // ============================================

  /**
   * Subscribe to live search results.
   * Returns initial results and tracks the subscription for delta updates.
   *
   * @param clientId - ID of the subscribing client
   * @param subscriptionId - Unique subscription identifier
   * @param mapName - Name of the map to search
   * @param query - Search query text
   * @param options - Search options (limit, minScore, boost)
   * @returns Initial search results
   */
  subscribe(
    clientId: string,
    subscriptionId: string,
    mapName: string,
    query: string,
    options?: SchemaSearchOptions
  ): ServerSearchResult[] {
    const index = this.indexes.get(mapName);

    if (!index) {
      logger.warn({ mapName }, 'Subscribe requested for map without FTS enabled');
      return [];
    }

    // Tokenize query ONCE using the index's tokenizer for consistency
    // This ensures the same tokenization is used for initial search and delta updates
    const queryTerms = index.tokenizeQuery(query);

    // Execute initial search
    const searchResults = index.search(query, options);

    // Build initial results and cache
    const currentResults = new Map<string, CachedResult>();
    const results: ServerSearchResult[] = [];

    for (const result of searchResults) {
      const value = this.getDocumentValue
        ? this.getDocumentValue(mapName, result.docId)
        : undefined;

      currentResults.set(result.docId, {
        score: result.score,
        matchedTerms: result.matchedTerms || [],
      });

      results.push({
        key: result.docId,
        value,
        score: result.score,
        matchedTerms: result.matchedTerms || [],
      });
    }

    // Create subscription
    const subscription: SearchSubscription = {
      id: subscriptionId,
      clientId,
      mapName,
      query,
      queryTerms,
      options: options || {},
      currentResults,
    };

    // Track subscription
    this.subscriptions.set(subscriptionId, subscription);

    // Track by map
    if (!this.subscriptionsByMap.has(mapName)) {
      this.subscriptionsByMap.set(mapName, new Set());
    }
    this.subscriptionsByMap.get(mapName)!.add(subscriptionId);

    // Track by client
    if (!this.subscriptionsByClient.has(clientId)) {
      this.subscriptionsByClient.set(clientId, new Set());
    }
    this.subscriptionsByClient.get(clientId)!.add(subscriptionId);

    logger.debug(
      { subscriptionId, clientId, mapName, query, resultCount: results.length },
      'Search subscription created'
    );

    return results;
  }

  /**
   * Unsubscribe from a live search.
   *
   * @param subscriptionId - Subscription to remove
   */
  unsubscribe(subscriptionId: string): void {
    const subscription = this.subscriptions.get(subscriptionId);
    if (!subscription) {
      return;
    }

    // Remove from subscriptions
    this.subscriptions.delete(subscriptionId);

    // Remove from map tracking
    const mapSubs = this.subscriptionsByMap.get(subscription.mapName);
    if (mapSubs) {
      mapSubs.delete(subscriptionId);
      if (mapSubs.size === 0) {
        this.subscriptionsByMap.delete(subscription.mapName);
      }
    }

    // Remove from client tracking
    const clientSubs = this.subscriptionsByClient.get(subscription.clientId);
    if (clientSubs) {
      clientSubs.delete(subscriptionId);
      if (clientSubs.size === 0) {
        this.subscriptionsByClient.delete(subscription.clientId);
      }
    }

    logger.debug({ subscriptionId }, 'Search subscription removed');
  }

  /**
   * Unsubscribe all subscriptions for a client.
   * Called when a client disconnects.
   *
   * @param clientId - ID of the disconnected client
   */
  unsubscribeClient(clientId: string): void {
    const clientSubs = this.subscriptionsByClient.get(clientId);
    if (!clientSubs) {
      return;
    }

    // Copy set since we're modifying during iteration
    const subscriptionIds = Array.from(clientSubs);
    for (const subscriptionId of subscriptionIds) {
      this.unsubscribe(subscriptionId);
    }

    logger.debug({ clientId, count: subscriptionIds.length }, 'Client subscriptions cleared');
  }

  /**
   * Get the number of active subscriptions.
   */
  getSubscriptionCount(): number {
    return this.subscriptions.size;
  }

  /**
   * Notify subscribers about a document change.
   * Computes delta (ENTER/UPDATE/LEAVE) for each affected subscription.
   *
   * @param mapName - Name of the map that changed
   * @param key - Document key that changed
   * @param value - New document value (null if removed)
   * @param changeType - Type of change
   */
  private notifySubscribers(
    mapName: string,
    key: string,
    value: Record<string, unknown> | null,
    changeType: 'add' | 'update' | 'remove'
  ): void {
    if (!this.sendUpdate) {
      return; // No callback registered
    }

    const subscriptionIds = this.subscriptionsByMap.get(mapName);
    if (!subscriptionIds || subscriptionIds.size === 0) {
      return; // No subscriptions for this map
    }

    const index = this.indexes.get(mapName);
    if (!index) {
      return; // No index (shouldn't happen)
    }

    for (const subId of subscriptionIds) {
      const sub = this.subscriptions.get(subId);
      if (!sub) continue;

      const wasInResults = sub.currentResults.has(key);
      let isInResults = false;
      let newScore = 0;
      let matchedTerms: string[] = [];

      logger.debug({ subId, key, wasInResults, changeType }, 'Processing subscription update');

      if (changeType !== 'remove' && value !== null) {
        // Re-score document against subscription query
        const result = this.scoreDocument(sub, key, value, index);
        if (result && result.score >= (sub.options.minScore ?? 0)) {
          isInResults = true;
          newScore = result.score;
          matchedTerms = result.matchedTerms;
        }
      }

      // Determine update type
      let updateType: SearchUpdateType | null = null;

      if (!wasInResults && isInResults) {
        updateType = 'ENTER';
        sub.currentResults.set(key, { score: newScore, matchedTerms });
      } else if (wasInResults && !isInResults) {
        updateType = 'LEAVE';
        sub.currentResults.delete(key);
      } else if (wasInResults && isInResults) {
        const old = sub.currentResults.get(key)!;
        // Send UPDATE if score changed OR if this is an update change type
        // (client should know document content changed even if score is same)
        if (Math.abs(old.score - newScore) > 0.0001 || changeType === 'update') {
          updateType = 'UPDATE';
          sub.currentResults.set(key, { score: newScore, matchedTerms });
        }
      }

      logger.debug({ subId, key, wasInResults, isInResults, updateType, newScore }, 'Update decision');

      if (updateType) {
        this.sendUpdate(
          sub.clientId,
          subId,
          key,
          value,
          newScore,
          matchedTerms,
          updateType
        );
      }
    }
  }

  /**
   * Score a single document against a subscription's query.
   *
   * OPTIMIZED: O(Q × D) complexity instead of O(N) full index scan.
   * Uses pre-tokenized queryTerms and FullTextIndex.scoreSingleDocument().
   *
   * @param subscription - The subscription containing query and cached queryTerms
   * @param key - Document key
   * @param value - Document value
   * @param index - The FullTextIndex for this map
   * @returns Scored result or null if document doesn't match
   */
  private scoreDocument(
    subscription: SearchSubscription,
    key: string,
    value: Record<string, unknown>,
    index: FullTextIndex
  ): { score: number; matchedTerms: string[] } | null {
    // Use O(1) single-document scoring with cached queryTerms
    const result = index.scoreSingleDocument(key, subscription.queryTerms, value);

    if (!result) {
      return null;
    }

    return {
      score: result.score,
      matchedTerms: result.matchedTerms || [],
    };
  }
}
