/**
 * SearchHandle - Client-side Live Search Subscription Handle
 *
 * Manages a live search subscription with delta updates.
 *
 * @module SearchHandle
 */

import type { SearchOptions, SearchUpdateType } from '@topgunbuild/core';
import type { SyncEngine, SearchResult } from './SyncEngine';

/**
 * Callback type for result change notifications.
 */
export type SearchResultsCallback<T> = (results: SearchResult<T>[]) => void;

/**
 * SearchHandle manages a live search subscription.
 *
 * Provides:
 * - Initial results on subscription
 * - Real-time delta updates (ENTER/UPDATE/LEAVE)
 * - Sorted results by relevance score
 * - Query update without re-subscribing
 *
 * @example
 * ```typescript
 * const handle = client.searchSubscribe<Article>('articles', 'machine learning', {
 *   limit: 20,
 *   minScore: 0.5
 * });
 *
 * // Subscribe to results
 * const unsubscribe = handle.subscribe((results) => {
 *   console.log('Results updated:', results.length);
 * });
 *
 * // Get current snapshot
 * const snapshot = handle.getResults();
 *
 * // Update query
 * handle.setQuery('deep learning');
 *
 * // Cleanup
 * handle.dispose();
 * ```
 */
export class SearchHandle<T = unknown> {
  /** Map name being searched */
  readonly mapName: string;

  /** Current search query */
  private _query: string;

  /** Search options */
  private _options?: SearchOptions;

  /** Unique subscription ID */
  private subscriptionId: string;

  /** Current results map (key → result) */
  private results: Map<string, SearchResult<T>> = new Map();

  /** Result change listeners */
  private listeners: Set<SearchResultsCallback<T>> = new Set();

  /** Whether the handle has been disposed */
  private disposed = false;

  /** Reference to SyncEngine */
  private syncEngine: SyncEngine;

  /** Handler for all messages (SEARCH_RESP and SEARCH_UPDATE) */
  private messageHandler: (message: any) => void;

  constructor(
    syncEngine: SyncEngine,
    mapName: string,
    query: string,
    options?: SearchOptions
  ) {
    this.syncEngine = syncEngine;
    this.mapName = mapName;
    this._query = query;
    this._options = options;
    this.subscriptionId = crypto.randomUUID();

    // Set up message handler that handles both SEARCH_RESP and SEARCH_UPDATE
    this.messageHandler = this.handleMessage.bind(this);

    // Register handler with SyncEngine
    this.syncEngine.on('message', this.messageHandler);

    // Send subscription request
    this.sendSubscribe();
  }

  /**
   * Handle incoming messages (both SEARCH_RESP and SEARCH_UPDATE).
   */
  private handleMessage(message: any): void {
    if (message.type === 'SEARCH_RESP') {
      this.handleSearchResponse(message);
    } else if (message.type === 'SEARCH_UPDATE') {
      this.handleSearchUpdate(message);
    }
  }

  /**
   * Get the current query string.
   */
  get query(): string {
    return this._query;
  }

  /**
   * Subscribe to result changes.
   * Callback is immediately called with current results.
   *
   * @param callback - Function called with updated results
   * @returns Unsubscribe function
   */
  subscribe(callback: SearchResultsCallback<T>): () => void {
    if (this.disposed) {
      throw new Error('SearchHandle has been disposed');
    }

    this.listeners.add(callback);

    // Immediately call with current results
    callback(this.getResults());

    return () => {
      this.listeners.delete(callback);
    };
  }

  /**
   * Get current results snapshot sorted by score (highest first).
   *
   * @returns Array of search results
   */
  getResults(): SearchResult<T>[] {
    return Array.from(this.results.values())
      .sort((a, b) => b.score - a.score);
  }

  /**
   * Get result count.
   */
  get size(): number {
    return this.results.size;
  }

  /**
   * Update the search query.
   * Triggers a new subscription with the updated query.
   *
   * @param query - New query string
   */
  setQuery(query: string): void {
    if (this.disposed) {
      throw new Error('SearchHandle has been disposed');
    }

    if (query === this._query) {
      return; // No change
    }

    // Unsubscribe from old query
    this.sendUnsubscribe();

    // Clear current results
    this.results.clear();

    // Update query and generate new subscription ID
    this._query = query;
    this.subscriptionId = crypto.randomUUID();

    // Subscribe to new query
    this.sendSubscribe();

    // Notify listeners of cleared results
    this.notifyListeners();
  }

  /**
   * Update search options.
   *
   * @param options - New search options
   */
  setOptions(options: SearchOptions): void {
    if (this.disposed) {
      throw new Error('SearchHandle has been disposed');
    }

    // Unsubscribe from old subscription
    this.sendUnsubscribe();

    // Clear current results
    this.results.clear();

    // Update options and generate new subscription ID
    this._options = options;
    this.subscriptionId = crypto.randomUUID();

    // Subscribe with new options
    this.sendSubscribe();

    // Notify listeners of cleared results
    this.notifyListeners();
  }

  /**
   * Dispose of the handle and cleanup resources.
   * After disposal, the handle cannot be used.
   */
  dispose(): void {
    if (this.disposed) {
      return;
    }

    this.disposed = true;

    // Send unsubscribe
    this.sendUnsubscribe();

    // Remove message handler
    this.syncEngine.off('message', this.messageHandler);

    // Clear state
    this.results.clear();
    this.listeners.clear();
  }

  /**
   * Check if handle is disposed.
   */
  isDisposed(): boolean {
    return this.disposed;
  }

  /**
   * Send SEARCH_SUB message to server.
   */
  private sendSubscribe(): void {
    this.syncEngine.send({
      type: 'SEARCH_SUB',
      payload: {
        subscriptionId: this.subscriptionId,
        mapName: this.mapName,
        query: this._query,
        options: this._options,
      },
    });
  }

  /**
   * Send SEARCH_UNSUB message to server.
   */
  private sendUnsubscribe(): void {
    this.syncEngine.send({
      type: 'SEARCH_UNSUB',
      payload: {
        subscriptionId: this.subscriptionId,
      },
    });
  }

  /**
   * Handle SEARCH_RESP message (initial results).
   */
  private handleSearchResponse(message: any): void {
    if (message.type !== 'SEARCH_RESP') return;
    if (message.payload?.requestId !== this.subscriptionId) return;

    const { results } = message.payload;

    if (Array.isArray(results)) {
      // Populate initial results
      for (const result of results) {
        this.results.set(result.key, {
          key: result.key,
          value: result.value as T,
          score: result.score,
          matchedTerms: result.matchedTerms || [],
        });
      }

      this.notifyListeners();
    }
  }

  /**
   * Handle SEARCH_UPDATE message (delta updates).
   */
  private handleSearchUpdate(message: any): void {
    if (message.type !== 'SEARCH_UPDATE') return;
    if (message.payload?.subscriptionId !== this.subscriptionId) return;

    const { key, value, score, matchedTerms, type } = message.payload;

    switch (type as SearchUpdateType) {
      case 'ENTER':
        // Document entered result set
        this.results.set(key, {
          key,
          value: value as T,
          score,
          matchedTerms: matchedTerms || [],
        });
        break;

      case 'UPDATE':
        // Document score changed
        const existing = this.results.get(key);
        if (existing) {
          existing.score = score;
          existing.matchedTerms = matchedTerms || [];
          // Value may have changed too
          existing.value = value as T;
        }
        break;

      case 'LEAVE':
        // Document left result set
        this.results.delete(key);
        break;
    }

    this.notifyListeners();
  }

  /**
   * Notify all listeners of result changes.
   */
  private notifyListeners(): void {
    const results = this.getResults();
    for (const listener of this.listeners) {
      try {
        listener(results);
      } catch (err) {
        console.error('SearchHandle listener error:', err);
      }
    }
  }
}
