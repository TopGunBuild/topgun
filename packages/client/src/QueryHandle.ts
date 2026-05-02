import { SyncEngine } from './SyncEngine';
import { ChangeTracker, ChangeEvent } from './ChangeTracker';
import { logger } from './utils/logger';
import type { PredicateNode } from '@topgunbuild/core';
import type { RecordSyncState } from './RecordSyncState';

export interface QueryFilter {
  where?: Record<string, any>;
  predicate?: PredicateNode;
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  /** Cursor for pagination */
  cursor?: string;
  /** Optional field projection — only these fields will be returned by the server */
  fields?: string[];
}

/** Cursor status for debugging */
export type CursorStatus = 'valid' | 'expired' | 'invalid' | 'none';

/** Pagination info from server */
export interface PaginationInfo {
  /** Cursor for fetching next page */
  nextCursor?: string;
  /** Whether more results are available */
  hasMore: boolean;
  /** Debug info: status of input cursor processing */
  cursorStatus: CursorStatus;
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

  // Change tracking for delta notifications
  private changeTracker = new ChangeTracker<T>();
  private pendingChanges: ChangeEvent<T>[] = [];
  private changeListeners: Set<(changes: ChangeEvent<T>[]) => void> = new Set();

  // Pagination info
  private _paginationInfo: PaginationInfo = { hasMore: false, cursorStatus: 'none' };
  private paginationListeners: Set<(info: PaginationInfo) => void> = new Set();

  // Per-record sync-state subscription. Lazily wired on first onSyncStateChange
  // or syncState read so QueryHandles that never observe syncState pay nothing.
  private syncStateListeners: Set<(snapshot: ReadonlyMap<string, RecordSyncState>) => void> = new Set();
  private syncStateUnsubscribe: (() => void) | null = null;
  private cachedSyncStateSnapshot: ReadonlyMap<string, RecordSyncState> | null = null;

  /** Field projection list — only these fields are returned by the server when set */
  public readonly fields: string[] | undefined;

  /** Merkle root hash from last server QUERY_RESP — used for delta reconnect */
  public merkleRootHash: number = 0;

  constructor(syncEngine: SyncEngine, mapName: string, filter: QueryFilter = {}) {
    this.id = crypto.randomUUID();
    this.syncEngine = syncEngine;
    this.mapName = mapName;
    this.filter = filter;
    this.fields = filter.fields;
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
  public onResult(items: { key: string, value: T }[], source: QueryResultSource = 'server', merkleRootHash?: number) {
    logger.debug({
      mapName: this.mapName,
      itemCount: items.length,
      source,
      currentResultsCount: this.currentResults.size,
      hasReceivedServerData: this.hasReceivedServerData
    }, 'QueryHandle onResult');

    // [FIX] Race condition protection for any async storage adapter:
    // If server sends empty QUERY_RESP before loading data from storage,
    // we ignore it to prevent clearing valid local data.
    // This is safe because:
    // 1. If server truly has no data, next non-empty response will clear local-only items
    // 2. If server is still loading, we preserve local data until real data arrives
    if (source === 'server' && items.length === 0 && !this.hasReceivedServerData) {
      logger.debug({ mapName: this.mapName }, 'QueryHandle ignoring empty server response - waiting for authoritative data');
      return;
    }

    // Mark that we've received authoritative server data (non-empty from server)
    if (source === 'server' && items.length > 0) {
      this.hasReceivedServerData = true;
    }

    // Store Merkle root hash for delta reconnect on next connection
    if (merkleRootHash !== undefined) {
      this.merkleRootHash = merkleRootHash;
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
      logger.debug({
        mapName: this.mapName,
        removedCount: removedKeys.length,
        removedKeys
      }, 'QueryHandle removed keys');
    }

    // Add/update new results
    for (const item of items) {
      this.currentResults.set(item.key, item.value);
    }
    logger.debug({
      mapName: this.mapName,
      resultCount: this.currentResults.size
    }, 'QueryHandle after merge');

    // Compute changes for delta tracking
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

    // Compute changes for delta tracking
    this.computeAndNotifyChanges(Date.now());

    this.notify();
  }

  /**
   * Subscribe to change events.
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
   * Get and clear pending changes.
   * Call this to retrieve all changes since the last consume.
   */
  public consumeChanges(): ChangeEvent<T>[] {
    const changes = [...this.pendingChanges];
    this.pendingChanges = [];
    return changes;
  }

  /**
   * Get last change without consuming.
   * Returns null if no pending changes.
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
        logger.error({ err: e }, 'QueryHandle change listener error');
      }
    }
  }

  private notify() {
    const results = this.getSortedResults();
    for (const listener of this.listeners) {
      listener(results);
    }
    // Result-set membership may have changed (keys added/removed). Re-evaluate
    // the filtered sync-state snapshot — only emits when content differs.
    this.notifySyncStateListenersIfChanged();
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

  // ============== Pagination Methods ==============

  /**
   * Get current pagination info.
   * Returns nextCursor, hasMore, and cursorStatus.
   */
  public getPaginationInfo(): PaginationInfo {
    return { ...this._paginationInfo };
  }

  /**
   * Subscribe to pagination info changes.
   * Called when server sends QUERY_RESP with new cursor info.
   *
   * @returns Unsubscribe function
   */
  public onPaginationChange(listener: (info: PaginationInfo) => void): () => void {
    this.paginationListeners.add(listener);
    // Immediately invoke with current value
    listener(this.getPaginationInfo());
    return () => this.paginationListeners.delete(listener);
  }

  /**
   * Update pagination info from server response.
   * Called by SyncEngine when processing QUERY_RESP.
   *
   * @internal
   */
  public updatePaginationInfo(info: Partial<PaginationInfo>): void {
    this._paginationInfo = {
      nextCursor: info.nextCursor,
      hasMore: info.hasMore ?? false,
      cursorStatus: info.cursorStatus ?? 'none',
    };
    this.notifyPaginationListeners();
  }

  private notifyPaginationListeners(): void {
    const info = this.getPaginationInfo();
    for (const listener of this.paginationListeners) {
      try {
        listener(info);
      } catch (e) {
        logger.error({ err: e }, 'QueryHandle pagination listener error');
      }
    }
  }

  // ============== Per-Record Sync State ==============

  /**
   * Snapshot of per-record sync-state for keys present in this query's result
   * set. Returns 'synced' (default) for keys with no opLog entry and no
   * rejection. Map identity is stable until at least one key in the result
   * set changes its projected state — safe to use as a useMemo / useEffect
   * dependency without churn.
   *
   * The marketing line behind this accessor: "Always know if your data has
   * hit the server."
   */
  public get syncState(): ReadonlyMap<string, RecordSyncState> {
    if (this.cachedSyncStateSnapshot) return this.cachedSyncStateSnapshot;
    const fresh = this.computeFilteredSyncState();
    this.cachedSyncStateSnapshot = fresh;
    return fresh;
  }

  /**
   * Subscribe to sync-state changes for keys in this query's result set.
   * Listener is invoked with a fresh snapshot only when at least one
   * relevant key's state changes — irrelevant per-record state changes for
   * other queries do NOT trigger re-renders here.
   *
   * Returns an unsubscribe function.
   */
  public onSyncStateChange(
    cb: (snapshot: ReadonlyMap<string, RecordSyncState>) => void,
  ): () => void {
    this.syncStateListeners.add(cb);
    this.ensureSyncStateSubscription();
    // Immediate emission with current snapshot — matches the existing
    // onPaginationChange / onChanges idioms.
    try {
      cb(this.syncState);
    } catch (e) {
      logger.error({ err: e }, 'QueryHandle syncState listener error (initial)');
    }
    return () => {
      this.syncStateListeners.delete(cb);
      if (this.syncStateListeners.size === 0) {
        this.teardownSyncStateSubscription();
      }
    };
  }

  /**
   * Called by onResult / onUpdate to re-evaluate the filtered snapshot when
   * the result set itself changes (keys added/removed). If the filtered
   * snapshot's content differs from the cached one, emit to listeners.
   */
  private notifySyncStateListenersIfChanged(): void {
    if (this.syncStateListeners.size === 0 && this.cachedSyncStateSnapshot === null) {
      return;
    }
    const fresh = this.computeFilteredSyncState();
    const prev = this.cachedSyncStateSnapshot;
    if (prev && this.syncStateMapsEqual(prev, fresh)) return;
    this.cachedSyncStateSnapshot = fresh;
    for (const listener of this.syncStateListeners) {
      try {
        listener(fresh);
      } catch (e) {
        logger.error({ err: e }, 'QueryHandle syncState listener error');
      }
    }
  }

  private ensureSyncStateSubscription(): void {
    if (this.syncStateUnsubscribe) return;
    const tracker = this.syncEngine.getRecordSyncStateTracker();
    this.syncStateUnsubscribe = tracker.onChange(this.mapName, () => {
      // Tracker fired for this map — recompute filtered snapshot. The
      // filtered snapshot may NOT change if the tracker change concerned a
      // key that's not in this query's result set; the equality check
      // suppresses the no-op emission.
      this.notifySyncStateListenersIfChanged();
    });
  }

  private teardownSyncStateSubscription(): void {
    if (!this.syncStateUnsubscribe) return;
    this.syncStateUnsubscribe();
    this.syncStateUnsubscribe = null;
    this.cachedSyncStateSnapshot = null;
  }

  private computeFilteredSyncState(): ReadonlyMap<string, RecordSyncState> {
    const tracker = this.syncEngine.getRecordSyncStateTracker();
    const out = new Map<string, RecordSyncState>();
    for (const key of this.currentResults.keys()) {
      out.set(key, tracker.get(this.mapName, key));
    }
    return out;
  }

  private syncStateMapsEqual(
    a: ReadonlyMap<string, RecordSyncState>,
    b: ReadonlyMap<string, RecordSyncState>,
  ): boolean {
    if (a.size !== b.size) return false;
    for (const [k, v] of a) {
      if (b.get(k) !== v) return false;
    }
    return true;
  }
}
