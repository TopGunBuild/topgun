import { SyncEngine } from './SyncEngine';
import { ChangeTracker, ChangeEvent } from './ChangeTracker';
import { logger } from './utils/logger';
import type { Aggregation, PredicateNode } from '@topgunbuild/core';
import type { RecordSyncState } from './RecordSyncState';

export interface QueryFilter {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- equality filters compare field values of unknown type; narrowing would require a generic on QueryFilter
  where?: Record<string, any>;
  predicate?: PredicateNode;
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  /** Cursor for pagination */
  cursor?: string;
  /** Optional field projection — only these fields will be returned by the server */
  fields?: string[];
  /** GROUP BY columns. Results carry one row per group with `__count` and any requested aggregate keys. */
  groupBy?: string[];
  /**
   * Field aggregations to compute per group (alongside the implicit `__count`).
   * Each requested `{ func, field }` surfaces as a `__<func>_<field>` key on the result row
   * (e.g. `{ func: 'sum', field: 'price' }` → `__sum_price`). Unrequested functions are not emitted.
   */
  aggregations?: Aggregation[];
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

/**
 * Per-emission metadata passed as the optional 2nd argument to a subscribe
 * callback.
 *
 * `settled` reflects the query-level latch: `false` while only local/optimistic
 * data has been delivered, `true` once the server has answered with a
 * QUERY_RESP for this query (including an empty result set). This lets a
 * subscriber distinguish "we're still waiting on the server" from "the server
 * has spoken and there genuinely are no rows".
 */
export type SubscribeMeta = { settled: boolean };

/**
 * Subscribe callback. The 2nd `meta` argument is optional so existing
 * single-arg `(results) => void` callbacks continue to type-check unchanged.
 */
export type SubscribeCallback<T> = (results: QueryResultItem<T>[], meta?: SubscribeMeta) => void;

export class QueryHandle<T> {
  public readonly id: string;
  private syncEngine: SyncEngine;
  private mapName: string;
  private filter: QueryFilter;
  private listeners: Set<SubscribeCallback<T>> = new Set();
  private currentResults: Map<string, T> = new Map();

  // Intent flag distinguishing a live-window handle (render-time top-N clamp
  // applies) from a page-accumulation handle. Once loadMore() runs, the handle
  // is permanently in page-accumulation mode and the clamp must NOT apply,
  // otherwise accumulated pages beyond `limit` would be hidden.
  private _paginated: boolean = false;

  // Change tracking for delta notifications
  private changeTracker = new ChangeTracker<T>();
  private pendingChanges: ChangeEvent<T>[] = [];
  private changeListeners: Set<(changes: ChangeEvent<T>[]) => void> = new Set();

  // Pagination info
  private _paginationInfo: PaginationInfo = { hasMore: false, cursorStatus: 'none' };
  private paginationListeners: Set<(info: PaginationInfo) => void> = new Set();

  // Per-record sync-state subscription. Lazily wired on first onSyncStateChange
  // or syncState read so QueryHandles that never observe syncState pay nothing.
  private syncStateListeners: Set<(snapshot: ReadonlyMap<string, RecordSyncState>) => void> =
    new Set();
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

  public subscribe(callback: SubscribeCallback<T>): () => void {
    this.listeners.add(callback);

    // If this is the first listener, activate subscription
    if (this.listeners.size === 1) {
      this.syncEngine.subscribeToQuery(this);
    } else {
      // Immediately invoke with cached results, carrying the current settled
      // state so a late subscriber sees the same { settled } it would have on
      // the next notify().
      callback(this.getSortedResults(), { settled: this.settled });
    }

    // [FIX]: Attempt to load local results immediately if available
    // This ensures that if data is already in storage but sync hasn't happened,
    // we still show something.
    this.loadInitialLocalData().then((data) => {
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

  // Settled latch: flips true on the FIRST server QUERY_RESP (even an empty
  // one). "Settled" means the server has spoken authoritatively for this query
  // — not that it returned rows. queryOnce and the { settled } subscribe option
  // read this single internal signal.
  private settled: boolean = false;
  private settledResolve: (() => void) | null = null;
  private settledPromise: Promise<void> = new Promise((resolve) => {
    this.settledResolve = resolve;
  });

  /**
   * True once the first server QUERY_RESP has arrived for this query (even an
   * empty result set). Distinct from "has rows" — an empty authoritative server
   * response still settles the query.
   *
   * @internal
   */
  public get isSettled(): boolean {
    return this.settled;
  }

  /**
   * Resolves when the first server QUERY_RESP arrives (settlement). If already
   * settled, resolves immediately. One-shot — subsequent settlements re-use the
   * already-resolved promise. Consumed by queryOnce and the { settled }
   * subscribe option.
   *
   * @internal
   */
  public whenSettled(): Promise<void> {
    return this.settledPromise;
  }

  /** Flip the settled latch and release any awaiters. Idempotent. */
  private markSettled(): void {
    if (this.settled) return;
    this.settled = true;
    this.settledResolve?.();
    this.settledResolve = null;
  }

  /**
   * Called by SyncEngine when server sends initial results or by local storage load.
   * Uses merge strategy instead of clear to prevent UI flickering.
   *
   * @param items - Array of key-value pairs
   * @param source - 'local' for IndexedDB data, 'server' for QUERY_RESP from server
   *
   * Settlement semantics:
   * - The first 'server' QUERY_RESP settles the query, even when empty. An empty
   *   authoritative response then clears stale local-only rows via the removed-key
   *   diff below — the server is the source of truth once it has spoken.
   * - 'local' results (loadInitialLocalData pre-load) NEVER settle the query and
   *   never clear data on emptiness; they only seed the cache before the server
   *   responds, so offline writes stay visible until a real QUERY_RESP arrives.
   */
  public onResult(
    items: { key: string; value: T }[],
    source: QueryResultSource = 'server',
    merkleRootHash?: number,
  ) {
    logger.debug(
      {
        mapName: this.mapName,
        itemCount: items.length,
        source,
        currentResultsCount: this.currentResults.size,
        settled: this.settled,
      },
      'QueryHandle onResult',
    );

    // Any server response is authoritative and settles the query — including an
    // empty result, which legitimately means "the server has no rows for this
    // query". Driven ONLY from the server source so the local pre-load artifact
    // (loadInitialLocalData) can never settle or clear data prematurely.
    if (source === 'server') {
      this.markSettled();
    }

    // Store Merkle root hash for delta reconnect on next connection
    if (merkleRootHash !== undefined) {
      this.merkleRootHash = merkleRootHash;
    }

    const newKeys = new Set(items.map((i) => i.key));

    // Remove only keys that are not in the new results
    const removedKeys: string[] = [];
    for (const key of this.currentResults.keys()) {
      if (!newKeys.has(key)) {
        removedKeys.push(key);
        this.currentResults.delete(key);
      }
    }
    if (removedKeys.length > 0) {
      logger.debug(
        {
          mapName: this.mapName,
          removedCount: removedKeys.length,
          removedKeys,
        },
        'QueryHandle removed keys',
      );
    }

    // Add/update new results
    for (const item of items) {
      this.currentResults.set(item.key, item.value);
    }
    logger.debug(
      {
        mapName: this.mapName,
        resultCount: this.currentResults.size,
      },
      'QueryHandle after merge',
    );

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
   * Subscribe to delta change events (add / update / remove per record).
   * For the full result set, use subscribe(). Returns an unsubscribe function.
   *
   * @example
   * ```typescript
   * const unsubscribe = handle.onDelta((changes) => {
   *   for (const change of changes) {
   *     if (change.type === 'add') {
   *       console.log('Added:', change.key, change.value);
   *     }
   *   }
   * });
   * ```
   */
  public onDelta(listener: (changes: ChangeEvent<T>[]) => void): () => void {
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
    // Snapshot the latch once per emission so every subscriber in this pass
    // observes the same settled value.
    const meta: SubscribeMeta = { settled: this.settled };
    for (const listener of this.listeners) {
      // Isolate each subscriber: a throwing subscriber must not block delivery
      // to later subscribers or propagate back into onResult/onUpdate.
      try {
        listener(results, meta);
      } catch (e) {
        logger.error({ err: e }, 'QueryHandle result listener error');
      }
    }
    // Result-set membership may have changed (keys added/removed). Re-evaluate
    // the filtered sync-state snapshot — only emits when content differs.
    this.notifySyncStateListenersIfChanged();
  }

  private getSortedResults(): (T & { _key: string })[] {
    // Include _key in each result for client-side matching/lookup
    const results = Array.from(this.currentResults.entries()).map(
      ([key, value]) => ({ ...(value as object), _key: key }) as T & { _key: string },
    );

    if (this.filter.sort) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- sort comparator parameters typed as any to allow dynamic field access by runtime string key
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

    // Render-time top-N window clamp. Keeps the HEAD of the sort-ordered array
    // (the N rows the active comparator ranks first). This is a non-destructive
    // view slice — currentResults is never mutated — so it composes with the
    // server's displacement LEAVE retractions without double-dropping rows and
    // breaking the net-N invariant. Skipped once the handle is paginated, since
    // loadMore intentionally accumulates rows beyond `limit`.
    if (
      !this._paginated &&
      Number.isInteger(this.filter.limit) &&
      (this.filter.limit as number) > 0
    ) {
      return results.slice(0, this.filter.limit);
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
   * In-flight cursor token for loadMore. Stores the cursor being fetched so
   * concurrent calls with the same cursor are deduplicated to a single request.
   */
  private _loadMoreInFlight: string | null = null;

  /**
   * Append-only merge: adds/updates keys from the page batch without removing
   * any existing keys. This preserves results from prior pages that are absent
   * from the new page batch — a full-set reconciliation via onResult would
   * prune those prior-page rows.
   */
  private mergePageResults(items: { key: string; value: T }[]): void {
    // Entering page-accumulation mode permanently disables the live-window
    // top-N clamp so accumulated pages beyond `limit` remain visible.
    this._paginated = true;
    for (const item of items) {
      this.currentResults.set(item.key, item.value);
    }
    this.computeAndNotifyChanges(Date.now());
    this.notify();
  }

  /**
   * Load the next page of results and append them to the current result set.
   *
   * Uses the cursor from the most recent server response. If no further pages
   * are available (`hasMore` is false) or a request for the same cursor is
   * already in flight, resolves immediately without issuing a duplicate request.
   *
   * Results from the new page are merged with the existing result set using an
   * append-only strategy: prior-page rows are never removed.
   */
  public async loadMore(): Promise<void> {
    const { nextCursor, hasMore } = this._paginationInfo;

    // No more pages available — nothing to fetch.
    if (!hasMore || !nextCursor) return;

    // Deduplicate concurrent calls for the same cursor.
    if (this._loadMoreInFlight === nextCursor) return;

    this._loadMoreInFlight = nextCursor;

    try {
      // Create a temporary one-shot handle with the next cursor, exactly
      // mirroring the queryOnce pattern: subscribe → settle → unsubscribe.
      const tempHandle = new QueryHandle<T>(this.syncEngine, this.mapName, {
        ...this.filter,
        cursor: nextCursor,
      });

      const pageItems: { key: string; value: T }[] = [];

      const unsub = tempHandle.subscribe((results) => {
        pageItems.length = 0;
        for (const item of results) {
          // Destructure the synthetic _key field added by getSortedResults.
          const { _key, ...rest } = item as T & { _key: string };
          pageItems.push({ key: _key, value: rest as T });
        }
      });

      await tempHandle.whenSettled();
      unsub();

      // Append-only merge: preserves rows from all prior pages.
      this.mergePageResults(pageItems);

      // Advance pagination state to reflect the new page's cursor/hasMore.
      const newPaginationInfo = tempHandle.getPaginationInfo();
      this._paginationInfo = {
        nextCursor: newPaginationInfo.nextCursor,
        hasMore: newPaginationInfo.hasMore,
        cursorStatus: newPaginationInfo.cursorStatus,
      };
      this.notifyPaginationListeners();
    } finally {
      this._loadMoreInFlight = null;
    }
  }

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
    // onPaginationChange / onDelta idioms.
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
