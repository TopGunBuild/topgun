/**
 * HybridQueryHandle - Query handle for hybrid FTS + filter queries
 *
 * Extends QueryHandle functionality to support:
 * - FTS predicates (match, matchPhrase, matchPrefix)
 * - Score-based sorting (_score field)
 * - Hybrid queries combining FTS with traditional filters
 *
 * Part of Phase 12: Unified Search
 *
 * @module HybridQueryHandle
 */

import { SyncEngine } from './SyncEngine';
import { ChangeTracker, ChangeEvent } from './ChangeTracker';
import { logger } from './utils/logger';
import type { PredicateNode } from '@topgunbuild/core';

/**
 * Filter options for hybrid queries.
 */
export interface HybridQueryFilter {
  /** Traditional where clause filters */
  where?: Record<string, any>;
  /** Predicate tree (can include FTS predicates) */
  predicate?: PredicateNode;
  /** Sort configuration - use '_score' for FTS relevance sorting */
  sort?: Record<string, 'asc' | 'desc'>;
  /** Maximum results */
  limit?: number;
  /** Skip N results */
  offset?: number;
}

/**
 * Result item with score for hybrid queries.
 */
export interface HybridResultItem<T> {
  /** The document */
  value: T;
  /** Unique key */
  _key: string;
  /** Relevance score (only for FTS queries) */
  _score?: number;
  /** Matched terms (only for FTS queries) */
  _matchedTerms?: string[];
}

/**
 * Source of query results.
 */
export type HybridResultSource = 'local' | 'server';

/**
 * HybridQueryHandle manages hybrid queries that combine FTS with filters.
 *
 * @example
 * ```typescript
 * // Create hybrid query: FTS + filter
 * const handle = new HybridQueryHandle(syncEngine, 'articles', {
 *   predicate: Predicates.and(
 *     Predicates.match('body', 'machine learning'),
 *     Predicates.equal('category', 'tech')
 *   ),
 *   sort: { _score: 'desc' },
 *   limit: 20
 * });
 *
 * // Subscribe to results
 * handle.subscribe((results) => {
 *   results.forEach(r => console.log(`${r._key}: ${r._score}`));
 * });
 * ```
 */
export class HybridQueryHandle<T> {
  public readonly id: string;
  private syncEngine: SyncEngine;
  private mapName: string;
  private filter: HybridQueryFilter;
  private listeners: Set<(results: HybridResultItem<T>[]) => void> = new Set();
  private currentResults: Map<string, { value: T; score?: number; matchedTerms?: string[] }> =
    new Map();

  // Change tracking
  private changeTracker = new ChangeTracker<T>();
  private pendingChanges: ChangeEvent<T>[] = [];
  private changeListeners: Set<(changes: ChangeEvent<T>[]) => void> = new Set();

  // Track server data reception
  private hasReceivedServerData: boolean = false;

  constructor(syncEngine: SyncEngine, mapName: string, filter: HybridQueryFilter = {}) {
    this.id = crypto.randomUUID();
    this.syncEngine = syncEngine;
    this.mapName = mapName;
    this.filter = filter;
  }

  /**
   * Subscribe to query results.
   */
  public subscribe(callback: (results: HybridResultItem<T>[]) => void): () => void {
    this.listeners.add(callback);

    // Activate subscription on first listener
    if (this.listeners.size === 1) {
      this.syncEngine.subscribeToHybridQuery(this);
    } else {
      // Return cached results immediately
      callback(this.getSortedResults());
    }

    // Load initial local data
    this.loadInitialLocalData().then((data) => {
      if (this.currentResults.size === 0) {
        this.onResult(data, 'local');
      }
    });

    return () => {
      this.listeners.delete(callback);
      if (this.listeners.size === 0) {
        this.syncEngine.unsubscribeFromHybridQuery(this.id);
      }
    };
  }

  private async loadInitialLocalData(): Promise<
    Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>
  > {
    // Use SyncEngine to run local hybrid query
    return this.syncEngine.runLocalHybridQuery(this.mapName, this.filter);
  }

  /**
   * Called by SyncEngine with query results.
   */
  public onResult(
    items: Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>,
    source: HybridResultSource = 'server'
  ): void {
    logger.debug(
      {
        mapName: this.mapName,
        itemCount: items.length,
        source,
        currentResultsCount: this.currentResults.size,
        hasReceivedServerData: this.hasReceivedServerData,
      },
      'HybridQueryHandle onResult'
    );

    // Race condition protection (same as QueryHandle)
    if (source === 'server' && items.length === 0 && !this.hasReceivedServerData) {
      logger.debug(
        { mapName: this.mapName },
        'HybridQueryHandle ignoring empty server response'
      );
      return;
    }

    if (source === 'server' && items.length > 0) {
      this.hasReceivedServerData = true;
    }

    const newKeys = new Set(items.map((i) => i.key));

    // Remove keys not in new results
    for (const key of this.currentResults.keys()) {
      if (!newKeys.has(key)) {
        this.currentResults.delete(key);
      }
    }

    // Add/update new results
    for (const item of items) {
      this.currentResults.set(item.key, {
        value: item.value,
        score: item.score,
        matchedTerms: item.matchedTerms,
      });
    }

    // Compute changes for delta tracking
    this.computeAndNotifyChanges(Date.now());

    this.notify();
  }

  /**
   * Called by SyncEngine on live update.
   */
  public onUpdate(
    key: string,
    value: T | null,
    score?: number,
    matchedTerms?: string[]
  ): void {
    if (value === null) {
      this.currentResults.delete(key);
    } else {
      this.currentResults.set(key, { value, score, matchedTerms });
    }

    this.computeAndNotifyChanges(Date.now());
    this.notify();
  }

  /**
   * Subscribe to change events.
   */
  public onChanges(listener: (changes: ChangeEvent<T>[]) => void): () => void {
    this.changeListeners.add(listener);
    return () => this.changeListeners.delete(listener);
  }

  /**
   * Get and clear pending changes.
   */
  public consumeChanges(): ChangeEvent<T>[] {
    const changes = [...this.pendingChanges];
    this.pendingChanges = [];
    return changes;
  }

  /**
   * Get last change without consuming.
   */
  public getLastChange(): ChangeEvent<T> | null {
    return this.pendingChanges.length > 0
      ? this.pendingChanges[this.pendingChanges.length - 1]
      : null;
  }

  /**
   * Get all pending changes without consuming.
   */
  public getPendingChanges(): ChangeEvent<T>[] {
    return [...this.pendingChanges];
  }

  /**
   * Clear all pending changes.
   */
  public clearChanges(): void {
    this.pendingChanges = [];
  }

  /**
   * Reset change tracker.
   */
  public resetChangeTracker(): void {
    this.changeTracker.reset();
    this.pendingChanges = [];
  }

  private computeAndNotifyChanges(timestamp: number): void {
    const dataMap = new Map<string, T>();
    for (const [key, entry] of this.currentResults) {
      dataMap.set(key, entry.value);
    }
    const changes = this.changeTracker.computeChanges(dataMap, timestamp);

    if (changes.length > 0) {
      this.pendingChanges.push(...changes);
      this.notifyChangeListeners(changes);
    }
  }

  private notifyChangeListeners(changes: ChangeEvent<T>[]): void {
    for (const listener of this.changeListeners) {
      try {
        listener(changes);
      } catch (e) {
        logger.error({ err: e }, 'HybridQueryHandle change listener error');
      }
    }
  }

  private notify(): void {
    const results = this.getSortedResults();
    for (const listener of this.listeners) {
      listener(results);
    }
  }

  /**
   * Get sorted results with _key and _score.
   */
  private getSortedResults(): HybridResultItem<T>[] {
    const results: HybridResultItem<T>[] = Array.from(this.currentResults.entries()).map(
      ([key, entry]) => ({
        value: entry.value,
        _key: key,
        _score: entry.score,
        _matchedTerms: entry.matchedTerms,
      })
    );

    // Sort by configured sort fields
    if (this.filter.sort) {
      results.sort((a, b) => {
        for (const [field, direction] of Object.entries(this.filter.sort!)) {
          let valA: any;
          let valB: any;

          if (field === '_score') {
            // Special handling for _score
            valA = a._score ?? 0;
            valB = b._score ?? 0;
          } else if (field === '_key') {
            valA = a._key;
            valB = b._key;
          } else {
            valA = (a.value as any)[field];
            valB = (b.value as any)[field];
          }

          if (valA < valB) return direction === 'asc' ? -1 : 1;
          if (valA > valB) return direction === 'asc' ? 1 : -1;
        }
        return 0;
      });
    }

    // Apply offset and limit
    let sliced = results;
    if (this.filter.offset) {
      sliced = sliced.slice(this.filter.offset);
    }
    if (this.filter.limit) {
      sliced = sliced.slice(0, this.filter.limit);
    }

    return sliced;
  }

  /**
   * Get the filter configuration.
   */
  public getFilter(): HybridQueryFilter {
    return this.filter;
  }

  /**
   * Get the map name.
   */
  public getMapName(): string {
    return this.mapName;
  }

  /**
   * Check if this query contains FTS predicates.
   */
  public hasFTSPredicate(): boolean {
    return this.filter.predicate ? this.containsFTS(this.filter.predicate) : false;
  }

  private containsFTS(predicate: PredicateNode): boolean {
    if (predicate.op === 'match' || predicate.op === 'matchPhrase' || predicate.op === 'matchPrefix') {
      return true;
    }
    if (predicate.children) {
      return predicate.children.some((child) => this.containsFTS(child));
    }
    return false;
  }
}
