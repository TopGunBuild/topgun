import { SyncEngine } from './SyncEngine';
import { ChangeTracker, ChangeEvent } from './ChangeTracker';
import type { PredicateNode } from '@topgunbuild/core';

export interface QueryFilter {
  where?: Record<string, any>;
  predicate?: PredicateNode;
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  offset?: number;
}

/** Source of query results for proper handling of race conditions */
export type QueryResultSource = 'local' | 'server';

/** Result item with _key field for client-side lookups */
export type QueryResultItem<T> = T & { _key: string };

export class QueryHandle<T> {
  public readonly id: string;
  private syncEngine: SyncEngine;
  private mapName: string;
  private filter: QueryFilter;
  private listeners: Set<(results: QueryResultItem<T>[]) => void> = new Set();
  private currentResults: Map<string, T> = new Map();

  // Change tracking (Phase 5.1)
  private changeTracker = new ChangeTracker<T>();
  private pendingChanges: ChangeEvent<T>[] = [];
  private changeListeners: Set<(changes: ChangeEvent<T>[]) => void> = new Set();

  constructor(syncEngine: SyncEngine, mapName: string, filter: QueryFilter = {}) {
    this.id = crypto.randomUUID();
    this.syncEngine = syncEngine;
    this.mapName = mapName;
    this.filter = filter;
  }

  public subscribe(callback: (results: QueryResultItem<T>[]) => void): () => void {
    this.listeners.add(callback);

    // If this is the first listener, activate subscription
    if (this.listeners.size === 1) {
      this.syncEngine.subscribeToQuery(this);
    } else {
      // Immediately invoke with cached results
      callback(this.getSortedResults());
    }

    // [FIX]: Attempt to load local results immediately if available
    // This ensures that if data is already in storage but sync hasn't happened,
    // we still show something.
    this.loadInitialLocalData().then(data => {
      // If we haven't received server results yet (currentResults empty),
      // and we have local data OR it's just the initial load, we should notify.
      // Even if data is empty, we might want to tell the subscriber "nothing here yet".
      if (this.currentResults.size === 0) {
         this.onResult(data, 'local');
      }
    });

    return () => {
      this.listeners.delete(callback);
      if (this.listeners.size === 0) {
        this.syncEngine.unsubscribeFromQuery(this.id);
      }
    };
  }

  private async loadInitialLocalData() {
      // This requires SyncEngine to expose a method to query local storage
      // For now, we can't easily reach storageAdapter directly from here without leaking abstraction.
      // A better approach is for SyncEngine.subscribeToQuery to trigger a local load.
      return this.syncEngine.runLocalQuery(this.mapName, this.filter);
  }

  // Track if we've received authoritative server response
  private hasReceivedServerData: boolean = false;

  /**
   * Called by SyncEngine when server sends initial results or by local storage load.
   * Uses merge strategy instead of clear to prevent UI flickering.
   *
   * @param items - Array of key-value pairs
   * @param source - 'local' for IndexedDB data, 'server' for QUERY_RESP from server
   *
   * Race condition protection:
   * - Empty server responses are ignored until we receive non-empty server data
   * - This prevents clearing local data when server hasn't loaded from storage yet
   * - Works with any async storage adapter (PostgreSQL, SQLite, Redis, etc.)
   */
  public onResult(items: { key: string, value: T }[], source: QueryResultSource = 'server') {
    console.log(`[QueryHandle:${this.mapName}] onResult called with ${items.length} items`, {
      source,
      currentResultsCount: this.currentResults.size,
      newItemKeys: items.map(i => i.key),
      hasReceivedServerData: this.hasReceivedServerData
    });

    // [FIX] Race condition protection for any async storage adapter:
    // If server sends empty QUERY_RESP before loading data from storage,
    // we ignore it to prevent clearing valid local data.
    // This is safe because:
    // 1. If server truly has no data, next non-empty response will clear local-only items
    // 2. If server is still loading, we preserve local data until real data arrives
    if (source === 'server' && items.length === 0 && !this.hasReceivedServerData) {
      console.log(`[QueryHandle:${this.mapName}] Ignoring empty server response - waiting for authoritative data`);
      return;
    }

    // Mark that we've received authoritative server data (non-empty from server)
    if (source === 'server' && items.length > 0) {
      this.hasReceivedServerData = true;
    }

    const newKeys = new Set(items.map(i => i.key));

    // Remove only keys that are not in the new results
    const removedKeys: string[] = [];
    for (const key of this.currentResults.keys()) {
      if (!newKeys.has(key)) {
        removedKeys.push(key);
        this.currentResults.delete(key);
      }
    }
    if (removedKeys.length > 0) {
      console.log(`[QueryHandle:${this.mapName}] Removed ${removedKeys.length} keys:`, removedKeys);
    }

    // Add/update new results
    for (const item of items) {
      this.currentResults.set(item.key, item.value);
    }
    console.log(`[QueryHandle:${this.mapName}] After merge: ${this.currentResults.size} results`);

    // Compute changes for delta tracking (Phase 5.1)
    this.computeAndNotifyChanges(Date.now());

    this.notify();
  }

  /**
   * Called by SyncEngine when server sends a live update
   */
  public onUpdate(key: string, value: T | null) {
    if (value === null) {
      this.currentResults.delete(key);
    } else {
      this.currentResults.set(key, value);
    }

    // Compute changes for delta tracking (Phase 5.1)
    this.computeAndNotifyChanges(Date.now());

    this.notify();
  }

  /**
   * Subscribe to change events (Phase 5.1).
   * Returns an unsubscribe function.
   *
   * @example
   * ```typescript
   * const unsubscribe = handle.onChanges((changes) => {
   *   for (const change of changes) {
   *     if (change.type === 'add') {
   *       console.log('Added:', change.key, change.value);
   *     }
   *   }
   * });
   * ```
   */
  public onChanges(listener: (changes: ChangeEvent<T>[]) => void): () => void {
    this.changeListeners.add(listener);
    return () => this.changeListeners.delete(listener);
  }

  /**
   * Get and clear pending changes (Phase 5.1).
   * Call this to retrieve all changes since the last consume.
   */
  public consumeChanges(): ChangeEvent<T>[] {
    const changes = [...this.pendingChanges];
    this.pendingChanges = [];
    return changes;
  }

  /**
   * Get last change without consuming (Phase 5.1).
   * Returns null if no pending changes.
   */
  public getLastChange(): ChangeEvent<T> | null {
    return this.pendingChanges.length > 0
      ? this.pendingChanges[this.pendingChanges.length - 1]
      : null;
  }

  /**
   * Get all pending changes without consuming (Phase 5.1).
   */
  public getPendingChanges(): ChangeEvent<T>[] {
    return [...this.pendingChanges];
  }

  /**
   * Clear all pending changes (Phase 5.1).
   */
  public clearChanges(): void {
    this.pendingChanges = [];
  }

  /**
   * Reset change tracker (Phase 5.1).
   * Use when query filter changes or on reconnect.
   */
  public resetChangeTracker(): void {
    this.changeTracker.reset();
    this.pendingChanges = [];
  }

  private computeAndNotifyChanges(timestamp: number): void {
    const changes = this.changeTracker.computeChanges(this.currentResults, timestamp);

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
        console.error('[QueryHandle] Change listener error:', e);
      }
    }
  }

  private notify() {
    const results = this.getSortedResults();
    for (const listener of this.listeners) {
      listener(results);
    }
  }

  private getSortedResults(): (T & { _key: string })[] {
    // Include _key in each result for client-side matching/lookup
    const results = Array.from(this.currentResults.entries()).map(
      ([key, value]) => ({ ...(value as object), _key: key } as T & { _key: string })
    );

    if (this.filter.sort) {
      results.sort((a: any, b: any) => {
        for (const [field, direction] of Object.entries(this.filter.sort!)) {
          const valA = a[field];
          const valB = b[field];

          if (valA < valB) return direction === 'asc' ? -1 : 1;
          if (valA > valB) return direction === 'asc' ? 1 : -1;
        }
        return 0;
      });
    }

    return results;
  }

  public getFilter(): QueryFilter {
    return this.filter;
  }

  public getMapName(): string {
    return this.mapName;
  }
}
