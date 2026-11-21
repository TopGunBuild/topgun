import { SortedMap } from '@topgunbuild/core';
import { SyncEngine } from './SyncEngine';
import type { ChangeEvent } from './ChangeTracker';
import { deepEqual } from './utils/deepEqual';
import { logger } from './utils/logger';
import type { Aggregation, PredicateNode } from '@topgunbuild/core';
import type { RecordSyncState } from './RecordSyncState';
import { QueryError } from './errors/QueryError';

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
  /**
   * GROUP BY columns. Results carry one row per group with `__count` and any requested aggregate keys.
   * Only `queryOnce` / `queryOncePaged` accept it; a live `query()` throws `QueryError`.
   */
  groupBy?: string[];
  /**
   * Field aggregations to compute per group (alongside the implicit `__count`).
   * Each requested `{ func, field }` surfaces as a `__<func>_<field>` key on the result row
   * (e.g. `{ func: 'sum', field: 'price' }` → `__sum_price`). Unrequested functions are not emitted.
   * Only `queryOnce` / `queryOncePaged` accept it; a live `query()` throws `QueryError`.
   */
  aggregations?: Aggregation[];
}

/**
 * Deep copy of plain data: arrays and plain objects are copied, any other value is
 * kept as it is. Not `structuredClone`, which jsdom and older runtimes lack.
 */
function copyPlain<T>(value: T): T {
  if (Array.isArray(value)) return value.map(copyPlain) as T;
  if (value !== null && typeof value === 'object') {
    const proto = Object.getPrototypeOf(value);
    if (proto === Object.prototype || proto === null) {
      return Object.fromEntries(
        Object.entries(value).map(([key, item]) => [key, copyPlain(item)]),
      ) as T;
    }
  }
  return value;
}

/** Where a row sits in the result order: its sort-field values, then its key. */
interface SortKey {
  values: unknown[];
  key: string;
}

/** One row of the result set: its value, the row object handed out, its place in the order. */
interface Entry<T> {
  value: T;
  row: QueryResultItem<T>;
  sortKey: SortKey;
}

/**
 * Rank of a sort value, as the server's sort key ranks it: booleans, then
 * numbers, then strings, then everything else (null, absent, objects, arrays,
 * dates), which compare equal among themselves.
 */
function sortRank(value: unknown): number {
  switch (typeof value) {
    case 'boolean':
      return 0;
    case 'number':
    case 'bigint':
      return 1;
    case 'string':
      return 2;
    default:
      return 3;
  }
}

/**
 * Code-point order, the byte order of UTF-8 the server compares strings in.
 * UTF-16 code-unit order differs from it only where a surrogate meets a code
 * unit in U+E000..U+FFFF, so those are remapped before comparing.
 */
function compareCodePoints(a: string, b: string): number {
  if (a === b) return 0;
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    let x = a.charCodeAt(i);
    let y = b.charCodeAt(i);
    if (x !== y) {
      if (x >= 0xd800 && y >= 0xd800) {
        x = x <= 0xdfff ? x + 0x2000 : x - 0x800;
        y = y <= 0xdfff ? y + 0x2000 : y - 0x800;
      }
      return x < y ? -1 : 1;
    }
  }
  return a.length < b.length ? -1 : 1;
}

function compareSortValues(a: unknown, b: unknown): number {
  const ra = sortRank(a);
  const rb = sortRank(b);
  if (ra !== rb) return ra - rb;
  switch (ra) {
    case 0:
      return a === b ? 0 : a ? 1 : -1;
    case 1: {
      // NaN sorts after every other number and equals NaN, so the order stays total.
      const aNaN = typeof a === 'number' && Number.isNaN(a);
      const bNaN = typeof b === 'number' && Number.isNaN(b);
      if (aNaN || bNaN) return aNaN === bNaN ? 0 : aNaN ? 1 : -1;
      return (a as number) < (b as number) ? -1 : (a as number) > (b as number) ? 1 : 0;
    }
    case 2:
      return compareCodePoints(a as string, b as string);
    default:
      return 0;
  }
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
  private rows: Map<string, Entry<T>> = new Map();
  // Rows in the server's sort-key order, so a live update moves one row in
  // O(log N) instead of re-sorting the whole result set.
  private order: SortedMap<SortKey, Entry<T>>;
  private sortFields: [string, 'asc' | 'desc'][];

  // Intent flag distinguishing a live-window handle (render-time top-N clamp
  // applies) from a page-accumulation handle. Once loadMore() runs, the handle
  // is permanently in page-accumulation mode and the clamp must NOT apply,
  // otherwise accumulated pages beyond `limit` would be hidden.
  private _paginated: boolean = false;

  // Cached sorted-results snapshot for synchronous external-store reads
  // (useSyncExternalStore.getSnapshot). Rebuilt lazily only when the result
  // set actually changes. Returning a referentially-stable array between
  // mutations is what keeps React's useSyncExternalStore from looping;
  // building a fresh array on every read would.
  private resultsSnapshot: QueryResultItem<T>[] = [];
  private resultsSnapshotDirty: boolean = true;
  // True once the handle has produced ANY result emission (local or server) —
  // lets a synchronous consumer derive `loading` without waiting for the first
  // async notify.
  private hasEmitted: boolean = false;

  // Change tracking for delta notifications. pendingChanges is the pull queue,
  // read synchronously; deliveryBatch holds what onDelta listeners have not
  // been given yet.
  private pendingChanges: ChangeEvent<T>[] = [];
  private deliveryBatch: ChangeEvent<T>[] = [];
  private changeListeners: Set<(changes: ChangeEvent<T>[]) => void> = new Set();
  // Set by resetChangeTracker: the next change computation reports every row as added.
  private reportAllAsAdd: boolean = false;

  // A burst of live updates is delivered once per timer turn, so a subscriber
  // holding N rows pays one O(N) emission per turn rather than one per update.
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private emitOwed: boolean = false;
  private flushing: boolean = false;

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

  // A one-shot handle (subscribe → settle → unsubscribe): the server answers its
  // snapshot and registers no live subscription for it.
  private readonly snapshotOnly: boolean;

  /**
   * @param options - @internal `snapshotOnly` marks a one-shot handle; set only by
   *   `queryOnce`, `queryOncePaged` and `loadMore`.
   * @throws QueryError when a live handle carries a non-empty `groupBy` or
   *   `aggregations`: per-record live updates cannot keep grouped rows correct.
   */
  constructor(
    syncEngine: SyncEngine,
    mapName: string,
    filter: QueryFilter = {},
    options?: { snapshotOnly?: boolean },
  ) {
    this.snapshotOnly = options?.snapshotOnly === true;
    // The handle owns a copy: every (re)subscription re-sends it, so a caller that
    // mutates its own filter later could otherwise turn a live handle into a grouped one.
    this.filter = copyPlain(filter);
    if (!this.snapshotOnly && (this.filter.groupBy?.length || this.filter.aggregations?.length)) {
      throw new QueryError(
        `QUERY_LIVE_AGGREGATE_UNSUPPORTED: a live query on map '${mapName}' cannot carry groupBy or aggregations; run it as a one-shot query (queryOnce / queryOncePaged)`,
        'QUERY_LIVE_AGGREGATE_UNSUPPORTED',
        mapName,
      );
    }
    this.id = crypto.randomUUID();
    this.syncEngine = syncEngine;
    this.mapName = mapName;
    this.fields = this.filter.fields;
    this.sortFields = Object.entries(this.filter.sort ?? {});
    this.order = new SortedMap<SortKey, Entry<T>>(this.compareSortKeys);
  }

  public subscribe(callback: SubscribeCallback<T>): () => void {
    this.listeners.add(callback);

    // If this is the first listener, activate subscription
    if (this.listeners.size === 1) {
      this.syncEngine.subscribeToQuery(this);
    } else {
      // Immediately invoke with cached results, carrying the current settled
      // state so a late subscriber sees the same { settled } it would have on
      // the next emission.
      callback(this.getSnapshot().slice(), { settled: this.settled });
    }

    // [FIX]: Attempt to load local results immediately if available
    // This ensures that if data is already in storage but sync hasn't happened,
    // we still show something.
    this.loadInitialLocalData()
      .then((data) => {
        // If we haven't received server results yet (no rows),
        // and we have local data OR it's just the initial load, we should notify.
        // Even if data is empty, we might want to tell the subscriber "nothing here yet".
        if (this.rows.size === 0) {
          this.onResult(data, 'local');
        }
      })
      .catch((err) => {
        // Best-effort pre-load. Some clauses (groupBy/aggregations/cursor) cannot
        // be evaluated offline and reject by design — skip the local snapshot and
        // let the authoritative server result settle the query.
        logger.debug({ err }, 'local query pre-load skipped (not available offline)');
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
  private settledReject: ((err: QueryError) => void) | null = null;
  private settledPromise: Promise<void> = this.createSettledPromise();

  // The server's last error for this query, kept beside the rows: an error
  // response never replaces the rows the handle shows.
  private serverError: QueryError | null = null;
  private errorListeners: Set<(error: QueryError | null) => void> = new Set();

  private createSettledPromise(): Promise<void> {
    const promise = new Promise<void>((resolve, reject) => {
      this.settledResolve = resolve;
      this.settledReject = reject;
    });
    // A handle nobody awaits must not raise an unhandled rejection when an
    // error arrives before settlement; awaiters still see the rejection.
    promise.catch(() => {});
    return promise;
  }

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
   * already-resolved promise. Rejects with the QueryError when a server error
   * arrives before settlement. Once the error clears, later calls get a
   * resolved promise if the handle has settled, else a new promise that the
   * next settlement resolves.
   * Consumed by queryOnce, queryOncePaged, loadMore and the { settled }
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
    this.settledReject = null;
  }

  /**
   * The server's error for this query, or null. Rows are kept while it is set.
   */
  public getError(): QueryError | null {
    return this.serverError;
  }

  /**
   * Subscribe to the query's error. The listener is called at once with the
   * current value and again on every change (null when the error clears).
   *
   * @returns Unsubscribe function
   */
  public onError(listener: (error: QueryError | null) => void): () => void {
    this.errorListeners.add(listener);
    this.callErrorListener(listener, this.serverError);
    return () => this.errorListeners.delete(listener);
  }

  /**
   * Record a server error. Rows and result subscribers are untouched; an
   * awaiter of whenSettled() is rejected if the query has not settled.
   *
   * @internal
   */
  public setServerError(error: QueryError): void {
    this.serverError = error;
    if (!this.settled) {
      this.settledReject?.(error);
      this.settledResolve = null;
      this.settledReject = null;
    }
    this.notifyErrorListeners();
  }

  /**
   * Clear the server error, if one is set.
   *
   * @internal
   */
  public clearServerError(): void {
    if (this.serverError === null) return;
    this.serverError = null;
    // An error before settlement rejected the settled promise for good. Once it
    // clears, a settled handle answers whenSettled() at once, and an unsettled
    // one waits for the chain that cleared it.
    this.settledPromise = this.settled ? Promise.resolve() : this.createSettledPromise();
    this.notifyErrorListeners();
  }

  private notifyErrorListeners(): void {
    for (const listener of this.errorListeners) {
      this.callErrorListener(listener, this.serverError);
    }
  }

  private callErrorListener(
    listener: (error: QueryError | null) => void,
    error: QueryError | null,
  ): void {
    try {
      listener(error);
    } catch (e) {
      logger.error({ err: e }, 'QueryHandle error listener error');
    }
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
        currentResultsCount: this.rows.size,
        settled: this.settled,
      },
      'QueryHandle onResult',
    );

    // Any server response is authoritative and settles the query — including an
    // empty result, which legitimately means "the server has no rows for this
    // query". Driven ONLY from the server source so the local pre-load artifact
    // (loadInitialLocalData) can never settle or clear data prematurely.
    // Settling first is safe: awaiters resume in a microtask, after the
    // synchronous emission below.
    if (source === 'server') {
      this.markSettled();
    }

    // Store Merkle root hash for delta reconnect on next connection
    if (merkleRootHash !== undefined) {
      this.merkleRootHash = merkleRootHash;
    }

    const timestamp = Date.now();
    const changes: ChangeEvent<T>[] = [];
    for (const item of items) {
      const change = this.applyValue(item.key, item.value, timestamp);
      if (change) changes.push(change);
    }
    // Remove only keys that are not in the new results, after the adds and
    // updates, so the merge never shows an emptied set in between.
    const newKeys = new Set(items.map((i) => i.key));
    for (const key of this.rows.keys()) {
      if (!newKeys.has(key)) {
        const change = this.applyRemove(key, timestamp);
        if (change) changes.push(change);
      }
    }
    this.recordChanges(changes, timestamp);

    // A snapshot is delivered before onResult returns (whenSettled awaiters must
    // see its rows), always — "settled with zero rows" is an emission too.
    this.emitOwed = true;
    this.flush();
  }

  /**
   * Called by SyncEngine when server sends a live update. Costs O(log N); the
   * emission follows on the next timer turn, together with every other update
   * of this turn.
   */
  public onUpdate(key: string, value: T | null) {
    const timestamp = Date.now();
    const change =
      value === null ? this.applyRemove(key, timestamp) : this.applyValue(key, value, timestamp);
    // Emit when the rows changed even if nothing was recorded: after
    // resetChangeTracker, removing the last row records no add.
    if (this.recordChanges(change ? [change] : [], timestamp) || change !== null) {
      this.scheduleFlush();
    }
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
    // Deliver what onDelta listeners are owed first: a dropped remove would
    // leave a removed key in a consumer's own copy, which the full add that
    // follows cannot take out.
    this.flush();
    this.reportAllAsAdd = true;
    this.pendingChanges = [];
  }

  /** The order of two rows: field by field as the server's sort key, then the key. */
  private readonly compareSortKeys = (a: SortKey, b: SortKey): number => {
    for (let i = 0; i < this.sortFields.length; i++) {
      const c = compareSortValues(a.values[i], b.values[i]);
      if (c !== 0) return this.sortFields[i][1] === 'desc' ? -c : c;
    }
    return compareCodePoints(a.key, b.key);
  };

  private sortKeyOf(key: string, value: T): SortKey {
    const fields =
      value !== null && typeof value === 'object' ? (value as Record<string, unknown>) : undefined;
    return { values: this.sortFields.map(([field]) => fields?.[field]), key };
  }

  /** Sets one row; returns its change, or null when the value is deep-equal to the stored one. */
  private applyValue(key: string, value: T, timestamp: number): ChangeEvent<T> | null {
    const entry = this.rows.get(key);
    if (entry !== undefined && deepEqual(entry.value, value)) return null;
    // Built once per value and handed out by every emission until the value changes.
    const row = { ...(value as object), _key: key } as QueryResultItem<T>;
    if (entry === undefined) {
      const added: Entry<T> = { value, row, sortKey: this.sortKeyOf(key, value) };
      this.rows.set(key, added);
      this.order.set(added.sortKey, added);
      this.resultsSnapshotDirty = true;
      return { type: 'add', key, value, timestamp };
    }
    const previousValue = entry.value;
    const sortKey = this.sortKeyOf(key, value);
    if (this.compareSortKeys(entry.sortKey, sortKey) !== 0) {
      this.order.delete(entry.sortKey);
      entry.sortKey = sortKey;
      this.order.set(sortKey, entry);
    }
    entry.value = value;
    entry.row = row;
    this.resultsSnapshotDirty = true;
    return { type: 'update', key, value, previousValue, timestamp };
  }

  /** Removes one row; returns its change, or null when the key is not held. */
  private applyRemove(key: string, timestamp: number): ChangeEvent<T> | null {
    const entry = this.rows.get(key);
    if (entry === undefined) return null;
    this.rows.delete(key);
    this.order.delete(entry.sortKey);
    this.resultsSnapshotDirty = true;
    return { type: 'remove', key, previousValue: entry.value, timestamp };
  }

  /**
   * Queues the changes of one computation for the pull API and for onDelta.
   * After resetChangeTracker, the computation reports every row held as added
   * instead. Returns whether anything was recorded.
   */
  private recordChanges(changes: ChangeEvent<T>[], timestamp: number): boolean {
    if (this.reportAllAsAdd) {
      this.reportAllAsAdd = false;
      changes = [];
      for (const [key, entry] of this.rows) {
        changes.push({ type: 'add', key, value: entry.value, timestamp });
      }
    }
    for (const change of changes) {
      this.pendingChanges.push(change);
      this.deliveryBatch.push(change);
    }
    return changes.length > 0;
  }

  private scheduleFlush(): void {
    this.emitOwed = true;
    // A running flush delivers what its listeners record before it returns.
    if (this.flushTimer !== null || this.flushing) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      this.flush();
    }, 0);
  }

  /**
   * Delivers the undelivered changes to onDelta listeners, then the rows to
   * subscribe listeners, then the sync state. A flush reached from a listener
   * of a running flush only leaves its changes queued: the running one loops
   * until nothing is owed, so every listener sees the batches in order.
   */
  private flush(): void {
    if (this.flushTimer !== null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    if (this.flushing) return;
    this.flushing = true;
    try {
      while (this.deliveryBatch.length > 0 || this.emitOwed) {
        const batch = this.deliveryBatch;
        this.deliveryBatch = [];
        this.emitOwed = false;
        if (batch.length > 0) this.notifyChangeListeners(batch);
        this.emitResults();
        // Result-set membership may have changed (keys added/removed). Re-evaluate
        // the filtered sync-state snapshot — only emits when content differs.
        this.notifySyncStateListenersIfChanged();
      }
    } finally {
      this.flushing = false;
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

  private emitResults(): void {
    this.hasEmitted = true;
    if (this.listeners.size === 0) return;
    const results = this.getSnapshot();
    // Snapshot the latch once per emission so every subscriber in this pass
    // observes the same settled value.
    const meta: SubscribeMeta = { settled: this.settled };
    for (const listener of this.listeners) {
      // Isolate each subscriber: a throwing subscriber must not block delivery
      // to later subscribers or propagate back into onResult/onUpdate. Each
      // gets its own array, so one that sorts or pushes in place cannot change
      // getSnapshot() or what the others receive.
      try {
        listener(results.slice(), meta);
      } catch (e) {
        logger.error({ err: e }, 'QueryHandle result listener error');
      }
    }
  }

  /**
   * Synchronous, referentially-stable snapshot of the current sorted result
   * set. Designed for React 18 `useSyncExternalStore(subscribe, getSnapshot)`:
   * the returned array identity only changes when the result set actually
   * changes, so a render that reads this between mutations gets the same array
   * and React does not loop. It reflects live updates not yet emitted.
   * The array and its row objects are shared: do not mutate them.
   *
   * Returns whatever the handle currently holds — including any local
   * pre-loaded rows — so the FIRST render after subscribing reflects cached
   * data instead of an empty array. (loadInitialLocalData runs on subscribe;
   * if data is already cached it is reflected on the next getSnapshot.)
   */
  public getSnapshot(): QueryResultItem<T>[] {
    if (this.resultsSnapshotDirty) {
      // Render-time top-N window clamp: the HEAD of the order. A view only —
      // rows are never dropped — so it composes with the server's displacement
      // LEAVE retractions without double-dropping rows. Skipped once the handle
      // is paginated, since loadMore intentionally accumulates rows beyond `limit`.
      const limit =
        !this._paginated && Number.isInteger(this.filter.limit) && (this.filter.limit as number) > 0
          ? (this.filter.limit as number)
          : Infinity;
      const results: QueryResultItem<T>[] = [];
      if (limit > 0) {
        for (const entry of this.order.values()) {
          results.push(entry.row);
          if (results.length >= limit) break;
        }
      }
      this.resultsSnapshot = results;
      this.resultsSnapshotDirty = false;
    }
    return this.resultsSnapshot;
  }

  /**
   * Synchronous metadata for deriving `loading` without an async round-trip.
   * `settled` is the server-answered latch; `hasEmitted` is true once any
   * result (local or server) has been delivered. A consumer can show data
   * immediately when `hasEmitted` is true even before the server settles.
   */
  public getSnapshotMeta(): { settled: boolean; hasEmitted: boolean } {
    return { settled: this.settled, hasEmitted: this.hasEmitted };
  }

  /** Returns a copy, so mutating the result cannot change what the handle sends. */
  public getFilter(): QueryFilter {
    return copyPlain(this.filter);
  }

  /** @internal Whether this is a one-shot handle the server must not register. */
  public isSnapshotOnly(): boolean {
    return this.snapshotOnly;
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
    if (!this._paginated) {
      this._paginated = true;
      this.resultsSnapshotDirty = true;
    }
    const timestamp = Date.now();
    const changes: ChangeEvent<T>[] = [];
    for (const item of items) {
      const change = this.applyValue(item.key, item.value, timestamp);
      if (change) changes.push(change);
    }
    this.recordChanges(changes, timestamp);
    this.emitOwed = true;
    this.flush();
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
    let unsub: (() => void) | undefined;

    try {
      // Create a temporary one-shot handle with the next cursor, exactly
      // mirroring the queryOnce pattern: subscribe → settle → unsubscribe.
      const tempHandle = new QueryHandle<T>(
        this.syncEngine,
        this.mapName,
        { ...this.filter, cursor: nextCursor },
        { snapshotOnly: true },
      );

      const pageItems: { key: string; value: T }[] = [];

      unsub = tempHandle.subscribe((results) => {
        pageItems.length = 0;
        for (const item of results) {
          // Destructure the synthetic _key field every emitted row carries.
          const { _key, ...rest } = item as T & { _key: string };
          pageItems.push({ key: _key, value: rest as T });
        }
      });

      await tempHandle.whenSettled();

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
      // In the finally, so a rejected page (a server error) still unsubscribes
      // the temporary handle.
      unsub?.();
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
   * Called by every flush to re-evaluate the filtered snapshot when the result
   * set itself changes (keys added/removed). If the filtered
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
    for (const key of this.rows.keys()) {
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
