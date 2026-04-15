/**
 * HybridSearchHandle - Client-side Live Hybrid Search Subscription Handle
 *
 * Manages a live tri-hybrid search subscription with delta updates.
 * Mirrors SearchHandle structurally; substitutes hybrid-search message types
 * (HYBRID_SEARCH_SUB / HYBRID_SEARCH_RESP / HYBRID_SEARCH_UPDATE / HYBRID_SEARCH_UNSUB).
 *
 * @module HybridSearchHandle
 */

import { vectorToBytes } from '@topgunbuild/core';
import type { SearchMethodType } from '@topgunbuild/core';
import type { SyncEngine } from './SyncEngine';
import { logger } from './utils/logger';

/**
 * A single result entry from a live hybrid search subscription.
 */
export interface HybridSearchHandleResult<T = unknown> {
  key: string;
  score: number;
  methodScores: Partial<Record<SearchMethodType, number>>;
  value?: T;
}

/**
 * Callback type for result change notifications.
 */
export type HybridSearchResultsCallback<T> = (
  results: HybridSearchHandleResult<T>[]
) => void;

/**
 * Options for a live hybrid search subscription.
 */
export interface HybridSearchSubscribeOptions {
  /** Search methods to use. Default: ['fullText'] */
  methods?: SearchMethodType[];
  /** Number of top results to return. Default: 10 */
  k?: number;
  /** Query vector for the semantic leg (Float32Array or number[]). */
  queryVector?: Float32Array | number[];
  /** Server-side predicate filter. */
  predicate?: unknown;
  /** Whether to include full document value in results. */
  includeValue?: boolean;
  /** Minimum score threshold. */
  minScore?: number;
}

/**
 * HybridSearchHandle manages a live tri-hybrid search subscription.
 *
 * Provides:
 * - Initial result set (via HYBRID_SEARCH_RESP matching subscriptionId)
 * - Real-time delta updates (ENTER/UPDATE/LEAVE via HYBRID_SEARCH_UPDATE)
 * - Sorted results by relevance score
 * - Query / options update with automatic resubscription
 *
 * @example
 * ```typescript
 * const handle = client.hybridSearchSubscribe<Article>('articles', 'machine learning', {
 *   methods: ['fullText', 'semantic'],
 *   k: 20,
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
 * // Update query without re-creating the handle
 * handle.setQuery('deep learning');
 *
 * // Cleanup
 * handle.dispose();
 * ```
 */
export class HybridSearchHandle<T = unknown> {
  /** Map name being searched */
  readonly mapName: string;

  /** Current search query text */
  private _queryText: string;

  /** Search options */
  private _options?: HybridSearchSubscribeOptions;

  /** Unique subscription ID (regenerated on setQuery / setOptions) */
  private subscriptionId: string;

  /** Current results map (key → result) */
  private results: Map<string, HybridSearchHandleResult<T>> = new Map();

  /** Result change listeners */
  private listeners: Set<HybridSearchResultsCallback<T>> = new Set();

  /** Whether the handle has been disposed */
  private disposed = false;

  /** Reference to SyncEngine */
  private syncEngine: SyncEngine;

  /** Bound message handler (retained for off() symmetry) */
  private messageHandler: (message: any) => void;

  constructor(
    syncEngine: SyncEngine,
    mapName: string,
    queryText: string,
    options?: HybridSearchSubscribeOptions
  ) {
    this.syncEngine = syncEngine;
    this.mapName = mapName;
    this._queryText = queryText;
    this._options = options;
    this.subscriptionId = crypto.randomUUID();

    // Bind once so the same reference is used for on() / off()
    this.messageHandler = this.handleMessage.bind(this);

    // Register for all broadcast messages from SyncEngine
    this.syncEngine.on('message', this.messageHandler);

    // Send initial subscription request to server
    this.sendSubscribe();
  }

  // ============================================
  // Message Handling
  // ============================================

  /**
   * Dispatch incoming SyncEngine messages.
   * Handles HYBRID_SEARCH_RESP (initial snapshot) and HYBRID_SEARCH_UPDATE (deltas).
   * Silently ignores all other message types — other handles share the same channel.
   */
  private handleMessage(message: any): void {
    if (message.type === 'HYBRID_SEARCH_RESP') {
      this.handleSearchResponse(message);
    } else if (message.type === 'HYBRID_SEARCH_UPDATE') {
      this.handleSearchUpdate(message);
    }
    // All other message types are intentionally ignored — no warning emitted
    // because other handles (and one-shot HybridSearchClient) use the same broadcast.
  }

  /**
   * Handle HYBRID_SEARCH_RESP (initial result set).
   * The server sets requestId equal to the subscriptionId from the HYBRID_SEARCH_SUB
   * payload — this is how the handle identifies its own initial snapshot.
   */
  private handleSearchResponse(message: any): void {
    if (message.type !== 'HYBRID_SEARCH_RESP') return;
    if (message.payload?.requestId !== this.subscriptionId) return;

    const { results } = message.payload;

    if (Array.isArray(results)) {
      for (const result of results) {
        this.results.set(result.key, {
          key: result.key,
          score: result.score,
          methodScores: result.methodScores ?? {},
          ...(result.value !== undefined ? { value: result.value as T } : {}),
        });
      }

      this.notifyListeners();
    }
  }

  /**
   * Handle HYBRID_SEARCH_UPDATE (live delta).
   * ENTER adds a new key, UPDATE mutates score/methodScores/value, LEAVE removes the key.
   */
  private handleSearchUpdate(message: any): void {
    if (message.type !== 'HYBRID_SEARCH_UPDATE') return;
    if (message.payload?.subscriptionId !== this.subscriptionId) return;

    const { key, value, score, methodScores, changeType } = message.payload;

    switch (changeType as string) {
      case 'ENTER':
        this.results.set(key, {
          key,
          score,
          methodScores: methodScores ?? {},
          ...(value !== undefined ? { value: value as T } : {}),
        });
        break;

      case 'UPDATE': {
        const existing = this.results.get(key);
        if (existing) {
          existing.score = score;
          existing.methodScores = methodScores ?? {};
          if (value !== undefined) {
            existing.value = value as T;
          }
        }
        break;
      }

      case 'LEAVE':
        this.results.delete(key);
        break;
    }

    this.notifyListeners();
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Current query text.
   */
  get query(): string {
    return this._queryText;
  }

  /**
   * Number of results currently held.
   */
  get size(): number {
    return this.results.size;
  }

  /**
   * Subscribe to result changes.
   * The callback is immediately invoked with the current snapshot.
   *
   * @param callback - Called with updated results array whenever results change
   * @returns Unsubscribe function
   * @throws If handle has been disposed
   */
  subscribe(callback: HybridSearchResultsCallback<T>): () => void {
    if (this.disposed) {
      throw new Error('HybridSearchHandle has been disposed');
    }

    this.listeners.add(callback);

    // Deliver current snapshot immediately
    callback(this.getResults());

    return () => {
      this.listeners.delete(callback);
    };
  }

  /**
   * Get current results snapshot sorted by score descending.
   *
   * @returns Array of result entries, highest score first
   */
  getResults(): HybridSearchHandleResult<T>[] {
    return Array.from(this.results.values()).sort((a, b) => b.score - a.score);
  }

  /**
   * Update the search query text.
   * Sends HYBRID_SEARCH_UNSUB for the current subscriptionId, clears local results,
   * regenerates subscriptionId, then sends a fresh HYBRID_SEARCH_SUB.
   * Subscribers are notified with an empty result array before the new results arrive.
   *
   * No-op if queryText is unchanged.
   *
   * @param queryText - New query string
   * @throws If handle has been disposed
   */
  setQuery(queryText: string): void {
    if (this.disposed) {
      throw new Error('HybridSearchHandle has been disposed');
    }

    if (queryText === this._queryText) {
      return;
    }

    this.sendUnsubscribe();
    this.results.clear();
    this.subscriptionId = crypto.randomUUID();
    this._queryText = queryText;
    this.sendSubscribe();
    this.notifyListeners();
  }

  /**
   * Update the search options.
   * Triggers the same unsub / clear / resubscribe cycle as setQuery.
   *
   * @param options - New search options
   * @throws If handle has been disposed
   */
  setOptions(options: HybridSearchSubscribeOptions): void {
    if (this.disposed) {
      throw new Error('HybridSearchHandle has been disposed');
    }

    this.sendUnsubscribe();
    this.results.clear();
    this._options = options;
    this.subscriptionId = crypto.randomUUID();
    this.sendSubscribe();
    this.notifyListeners();
  }

  /**
   * Dispose the handle and release all resources.
   * Sends HYBRID_SEARCH_UNSUB, removes the SyncEngine message listener,
   * and clears results and listener sets. Idempotent — safe to call multiple times.
   */
  dispose(): void {
    if (this.disposed) {
      return;
    }

    this.disposed = true;
    this.sendUnsubscribe();
    this.syncEngine.off('message', this.messageHandler);
    this.results.clear();
    this.listeners.clear();
  }

  /**
   * Check if the handle has been disposed.
   */
  isDisposed(): boolean {
    return this.disposed;
  }

  // ============================================
  // Wire Protocol
  // ============================================

  /**
   * Send HYBRID_SEARCH_SUB to server.
   * Uses ternary-spread pattern (not ...(x ?? {})) to avoid accidentally
   * spreading Uint8Array enumerable properties into the payload object.
   */
  private sendSubscribe(): void {
    // Convert queryVector to little-endian wire bytes for the semantic leg when provided
    const queryVector =
      this._options?.queryVector !== undefined
        ? vectorToBytes(this._options.queryVector)
        : undefined;

    this.syncEngine.send({
      type: 'HYBRID_SEARCH_SUB',
      payload: {
        subscriptionId: this.subscriptionId,
        mapName: this.mapName,
        queryText: this._queryText,
        methods: this._options?.methods ?? ['fullText'],
        k: this._options?.k ?? 10,
        ...(queryVector !== undefined ? { queryVector } : {}),
        ...(this._options?.predicate !== undefined ? { predicate: this._options.predicate } : {}),
        ...(this._options?.includeValue !== undefined ? { includeValue: this._options.includeValue } : {}),
        ...(this._options?.minScore !== undefined ? { minScore: this._options.minScore } : {}),
      },
    });
  }

  /**
   * Send HYBRID_SEARCH_UNSUB to server.
   */
  private sendUnsubscribe(): void {
    this.syncEngine.send({
      type: 'HYBRID_SEARCH_UNSUB',
      payload: {
        subscriptionId: this.subscriptionId,
      },
    });
  }

  // ============================================
  // Internal Helpers
  // ============================================

  /**
   * Notify all subscribers with the current sorted result snapshot.
   * Errors thrown by individual listeners are caught and logged so one
   * misbehaving listener cannot silence the rest.
   */
  private notifyListeners(): void {
    const results = this.getResults();
    for (const listener of this.listeners) {
      try {
        listener(results);
      } catch (err) {
        logger.error(
          { err, mapName: this.mapName, context: 'listener' },
          'HybridSearchHandle listener error'
        );
      }
    }
  }
}
