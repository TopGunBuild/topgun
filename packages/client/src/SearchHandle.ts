/**
 * SearchHandle - Client-side Live Search Subscription Handle
 *
 * Manages a live search subscription with delta updates.
 *
 * @module SearchHandle
 */

import type { SearchOptions, SearchUpdateType } from '@topgunbuild/core';
import type { SyncEngine, SearchResult } from './SyncEngine';
import type { LiveSearchSubscription } from './sync/types';
import { logger } from './utils/logger';

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
export class SearchHandle<T = unknown> implements LiveSearchSubscription {
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

  // Deltas of the current subscription that arrived before its response. The server
  // registers a subscription before it answers, so a delta can overtake the response;
  // it is applied after the response replaces the rows. null: no response awaited.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- SEARCH_UPDATE payloads as received
  private pendingDeltas: any[] | null = null;

  // Cached sorted-results snapshot for synchronous external-store reads
  // (useSyncExternalStore.getSnapshot). Rebuilt lazily only when results change
  // (dirty flag flipped in notifyListeners). A referentially-stable array
  // between mutations is what keeps React's useSyncExternalStore from looping.
  private resultsSnapshot: SearchResult<T>[] = [];
  private resultsSnapshotDirty = true;

  /** Reference to SyncEngine */
  private syncEngine: SyncEngine;

  /** Handler for all messages (SEARCH_RESP and SEARCH_UPDATE) */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- SyncEngine message bus broadcasts heterogeneous message types; narrowed by message.type in handleMessage
  private messageHandler: (message: any) => void;

  constructor(syncEngine: SyncEngine, mapName: string, query: string, options?: SearchOptions) {
    this.syncEngine = syncEngine;
    this.mapName = mapName;
    this._query = query;
    this._options = options;
    this.subscriptionId = crypto.randomUUID();

    // Set up message handler that handles both SEARCH_RESP and SEARCH_UPDATE
    this.messageHandler = this.handleMessage.bind(this);

    // Register handler with SyncEngine
    this.syncEngine.on('message', this.messageHandler);

    // Subscribes now if authenticated, else on the next authenticated socket.
    this.syncEngine.subscribeToLiveSearch(this);
  }

  /**
   * Handle incoming messages (both SEARCH_RESP and SEARCH_UPDATE).
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- message is a deserialized msgpack object from SyncEngine broadcast; narrowed by message.type before dispatch
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
    const sorted = Array.from(this.results.values()).sort((a, b) => b.score - a.score);

    // Render-time top-N window clamp read from the current options each call, so
    // a re-setOptions with a new limit takes effect on the next emission. This is
    // a non-destructive view slice — this.results is never mutated — so it
    // composes with the server's displacement LEAVE retractions without
    // double-dropping rows and breaking the net-N invariant.
    const limit = this._options?.limit;
    if (Number.isInteger(limit) && (limit as number) > 0) {
      return sorted.slice(0, limit);
    }

    return sorted;
  }

  /**
   * Synchronous, referentially-stable snapshot of the current sorted results
   * for React 18 `useSyncExternalStore`. The array identity only changes when
   * results change (dirty flag flipped in notifyListeners), preventing render
   * loops. Reflects whatever results are currently cached, so the first render
   * after subscribing shows them synchronously.
   */
  getSnapshot(): SearchResult<T>[] {
    if (this.resultsSnapshotDirty) {
      this.resultsSnapshot = this.getResults();
      this.resultsSnapshotDirty = false;
    }
    return this.resultsSnapshot;
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

    // Subscribe to new query (on the next authenticated socket if not authenticated now)
    this.syncEngine.subscribeToLiveSearch(this);

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

    // Subscribe with new options (on the next authenticated socket if not authenticated now)
    this.syncEngine.subscribeToLiveSearch(this);

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

    this.syncEngine.unsubscribeFromLiveSearch(this);

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
   * Subscribe under a new subscription id. Called by the client on every newly
   * authenticated socket; the rows stay visible until the response replaces them.
   */
  resubscribe(): void {
    if (this.disposed) {
      return;
    }
    // A new id makes every frame of an earlier subscription inert.
    this.subscriptionId = crypto.randomUUID();
    this.pendingDeltas = [];
    this.sendSubscribe();
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- message is a deserialized msgpack object; payload fields are accessed after type guard on message.type
  private handleSearchResponse(message: any): void {
    if (message.type !== 'SEARCH_RESP') return;
    if (message.payload?.requestId !== this.subscriptionId) return;

    const { results, error } = message.payload;
    // Every response of the current subscription ends the wait for it.
    const held = this.pendingDeltas ?? [];
    this.pendingDeltas = null;

    // An error response registered nothing on the server: keep the rows as they are.
    if (error != null) {
      return;
    }

    if (Array.isArray(results)) {
      // The response is the whole result set: rows it lacks are gone.
      this.results.clear();
      for (const result of results) {
        this.results.set(result.key, {
          key: result.key,
          value: result.value as T,
          score: result.score,
          matchedTerms: result.matchedTerms || [],
        });
      }

      for (const delta of held) {
        this.applyDelta(delta);
      }

      this.notifyListeners();
    }
  }

  /**
   * Handle SEARCH_UPDATE message (delta updates).
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- message is a deserialized msgpack object; payload fields are accessed after type guard on message.type
  private handleSearchUpdate(message: any): void {
    if (message.type !== 'SEARCH_UPDATE') return;
    if (message.payload?.subscriptionId !== this.subscriptionId) return;

    if (this.pendingDeltas) {
      this.pendingDeltas.push(message.payload);
      return;
    }

    this.applyDelta(message.payload);
    this.notifyListeners();
  }

  /**
   * Apply one SEARCH_UPDATE payload to the result map.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- SEARCH_UPDATE payload as received
  private applyDelta(payload: any): void {
    const { key, value, score, matchedTerms, changeType } = payload;

    switch (changeType as SearchUpdateType) {
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
  }

  /**
   * Notify all listeners of result changes.
   */
  private notifyListeners(): void {
    // Results changed — invalidate the cached snapshot for getSnapshot().
    this.resultsSnapshotDirty = true;
    const results = this.getResults();
    for (const listener of this.listeners) {
      try {
        listener(results);
      } catch (err) {
        logger.error(
          { err, mapName: this.mapName, context: 'listener' },
          'SearchHandle listener error',
        );
      }
    }
  }
}
