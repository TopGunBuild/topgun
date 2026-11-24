/**
 * QueryManager - Manages query subscriptions and local query execution
 *
 * This module extracts query management from SyncEngine:
 * - Standard query subscriptions (QueryHandle)
 * - Hybrid query subscriptions (HybridQueryHandle with FTS)
 * - Local query execution against storage adapter
 */

import { evaluatePredicate } from '@topgunbuild/core';
import type { QueryRespPayload, QueryUpdatePayload } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import { QueryError } from '../errors/QueryError';
import type { QueryErrorCode } from '../errors/QueryError';
import type { QueryHandle, QueryFilter } from '../QueryHandle';
import type { HybridQueryHandle, HybridQueryFilter } from '../HybridQueryHandle';
import type { IQueryManager, LiveSearchSubscription, QueryManagerConfig } from './types';

const SNAPSHOT_OVERFLOW = 'QUERY_SNAPSHOT_OVERFLOW';

/** Resubscribes a live handle may send after snapshot overflows, per subscription chain. */
const MAX_OVERFLOW_RESUBSCRIBES = 3;

/** Delay before the first resubscribe; each later one waits four times longer. */
const OVERFLOW_RESUBSCRIBE_BASE_MS = 250;

const KNOWN_QUERY_ERROR_CODES: ReadonlySet<string> = new Set<QueryErrorCode>([
  'QUERY_LIVE_AGGREGATE_UNSUPPORTED',
  'QUERY_SNAPSHOT_OVERFLOW',
  'QUERY_FAILED',
]);

/**
 * Per-handle error bookkeeping. A subscription chain starts with a handle's
 * first QUERY_SUB on a connection or with the one resubscribeAll sends; the
 * overflow resubscribes below belong to the chain they were sent in.
 */
interface QueryErrorState {
  /** Incremented at every chain start; never reset, so "a later chain" stays comparable. */
  chain: number;
  /** Snapshot overflows seen in the current chain. */
  overflows: number;
  timer: ReturnType<typeof setTimeout> | undefined;
  /**
   * How the handle's current error was raised: `overflow` when the resubscribe
   * bound ran out (cleared at the next chain start), `terminal` for any other
   * error response (cleared by a clean response in a later chain).
   */
  errorKind: 'overflow' | 'terminal' | null;
  errorChain: number;
}

/**
 * What a live handle holds from the moment it sends QUERY_SUB until that
 * send's first answer: its QUERY_UPDATEs, and an overflow answer that arrived
 * before the snapshot it follows. The server may deliver a small message
 * before a large snapshot that was queued ahead of it (TG-WS-004); applying
 * the held messages after the snapshot gives the rows the order of the
 * server's queue.
 */
interface QueryHold {
  updates: QueryUpdatePayload[];
  overflow: QueryRespPayload | null;
}

/**
 * QueryManager handles all query-related operations for SyncEngine.
 * It owns the queries and hybridQueries Maps (single source of truth).
 */
export class QueryManager implements IQueryManager {
  private readonly config: QueryManagerConfig;

  /** Standard queries (single source of truth) */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased in the manager registry; typed handles are created in TopGunClient
  private queries: Map<string, QueryHandle<any>> = new Map();

  /** Hybrid queries with FTS support (single source of truth) */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased in the manager registry
  private hybridQueries: Map<string, HybridQueryHandle<any>> = new Map();

  /** Live search and live hybrid search handles, restored by resubscribeAll. */
  private liveSearches: Set<LiveSearchSubscription> = new Set();

  // Keyed by the handle so the chain number and the error kind live as long as
  // the handle does, across unsubscribe/subscribe cycles.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased in the manager registry
  private errorStates: WeakMap<QueryHandle<any>, QueryErrorState> = new WeakMap();

  /**
   * The id each QUERY_SUB carries is new at every send, so the frames of a
   * superseded send of the same handle carry an id nobody owns and are
   * ignored: a later snapshot may arrive before an earlier one (TG-WS-004).
   * `wireIds` maps a handle's id to its current send's id, `byWireId` the
   * reverse.
   */
  private wireIds: Map<string, string> = new Map();

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased in the manager registry
  private byWireId: Map<string, QueryHandle<any>> = new Map();

  /** The hold of each live handle whose current send has not been answered, by handle id. */
  private holds: Map<string, QueryHold> = new Map();

  constructor(config: QueryManagerConfig) {
    this.config = config;
  }

  // ============================================
  // Query Access Methods
  // ============================================

  /**
   * Get all queries (read-only access).
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the manager interface; individual handles are typed at creation
  public getQueries(): Map<string, QueryHandle<any>> {
    return this.queries;
  }

  /**
   * Get all hybrid queries.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the manager interface
  public getHybridQueries(): Map<string, HybridQueryHandle<any>> {
    return this.hybridQueries;
  }

  /**
   * Get a hybrid query by ID.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- hybrid query type parameter erased at lookup time; caller narrows the result
  public getHybridQuery(queryId: string): HybridQueryHandle<any> | undefined {
    return this.hybridQueries.get(queryId);
  }

  // ============================================
  // Standard Query Methods
  // ============================================

  /**
   * Subscribe to a standard query.
   * Adds to queries Map and sends subscription to server if authenticated.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at subscription time; actual type lives in the handle's internal state
  public subscribeToQuery(query: QueryHandle<any>): void {
    this.queries.set(query.id, query);
    if (this.config.isAuthenticated()) {
      // A second subscribe without an unsubscribe (StrictMode) would leave the
      // earlier send registered on the server under its own id.
      const previous = this.wireIds.get(query.id);
      if (previous !== undefined) {
        this.config.sendMessage({ type: 'QUERY_UNSUB', payload: { queryId: previous } });
      }
      this.startChain(query);
      this.sendQuerySubscription(query);
    }
  }

  /**
   * Register a live search handle.
   * It subscribes now if authenticated; otherwise the next restore subscribes it.
   */
  public subscribeToLiveSearch(handle: LiveSearchSubscription): void {
    this.liveSearches.add(handle);
    if (this.config.isAuthenticated()) {
      // A failed send leaves the handle registered: the next restore subscribes it.
      this.eachLogged([handle], (h) => h.resubscribe());
    }
  }

  /**
   * Remove a live search handle from the registry. The handle sends its own unsubscribe.
   */
  public unsubscribeFromLiveSearch(handle: LiveSearchSubscription): void {
    this.liveSearches.delete(handle);
  }

  /**
   * Unsubscribe from a query.
   * Removes from Map and sends unsubscription to server if authenticated.
   */
  public unsubscribeFromQuery(queryId: string): void {
    const query = this.queries.get(queryId);
    if (query) {
      const state = this.errorStates.get(query);
      if (state) {
        this.cancelTimer(state);
        state.overflows = 0;
      }
    }
    this.queries.delete(queryId);
    this.holds.delete(queryId);
    const wireId = this.wireIds.get(queryId);
    if (wireId === undefined) return;
    this.wireIds.delete(queryId);
    this.byWireId.delete(wireId);
    if (this.config.isAuthenticated()) {
      this.config.sendMessage({
        type: 'QUERY_UNSUB',
        payload: { queryId: wireId },
      });
    }
  }

  /**
   * The handle whose current QUERY_SUB send carried `wireId`; undefined for
   * an unknown id and for a superseded send's.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the routing layer
  public handleOf(wireId: string): QueryHandle<any> | undefined {
    return this.byWireId.get(wireId);
  }

  /**
   * Send query subscription message to server, under a new wire id; a live
   * handle's hold starts over, discarding what an earlier send held.
   * Includes field projection when specified in the query filter.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the routing layer; only the filter and map name are needed for the subscription message
  private sendQuerySubscription(query: QueryHandle<any>): string {
    const filter = query.getFilter();
    // Serialize sort as an ordered array so the Rust server receives Vec<SortField>
    // rather than a plain object (whose key iteration order is not guaranteed on the wire).
    // Sort order is defined by JavaScript object key insertion order — accepted by design.
    const wireQuery = filter.sort
      ? {
          ...filter,
          sort: Object.entries(filter.sort).map(([field, direction]) => ({ field, direction })),
        }
      : filter;
    const previous = this.wireIds.get(query.id);
    if (previous !== undefined) this.byWireId.delete(previous);
    const wireId = crypto.randomUUID();
    this.wireIds.set(query.id, wireId);
    this.byWireId.set(wireId, query);
    if (query.isSnapshotOnly()) {
      this.holds.delete(query.id);
    } else {
      this.holds.set(query.id, { updates: [], overflow: null });
    }
    this.config.sendMessage({
      type: 'QUERY_SUB',
      payload: {
        queryId: wireId,
        mapName: query.getMapName(),
        query: wireQuery,
        fields: filter.fields,
        // Only a one-shot handle carries the flag, so a live payload is unchanged.
        ...(query.isSnapshotOnly() ? { snapshotOnly: true } : {}),
      },
    });
    return wireId;
  }

  /**
   * Route a QUERY_RESP to its handle. An error response never reaches the
   * rows: a snapshot overflow on a live handle resubscribes (at most three
   * times per chain, with backoff), anything else raises a QueryError.
   */
  public handleQueryResp(payload: QueryRespPayload): void {
    const query = this.handleOf(payload.queryId);
    if (!query) return;

    const hold = this.holds.get(query.id);
    if (hold) {
      // The server sends an overflow answer right after its snapshot, so an
      // overflow seen first means the snapshot is still on its way.
      if (payload.code === SNAPSHOT_OVERFLOW && hold.overflow === null) {
        hold.overflow = payload;
        return;
      }
      this.holds.delete(query.id);
    }
    this.applyQueryResp(query, payload);
    if (hold) {
      for (const update of hold.updates) this.handleQueryUpdate(update);
      if (hold.overflow) this.handleQueryResp(hold.overflow);
    }
  }

  /**
   * Route a QUERY_UPDATE to its handle: held while the handle's current send
   * is unanswered, applied otherwise; an update of an unknown or superseded
   * send is ignored.
   */
  public handleQueryUpdate(payload: QueryUpdatePayload): void {
    const query = this.handleOf(payload.queryId);
    if (!query) return;
    const hold = this.holds.get(query.id);
    if (hold) {
      hold.updates.push(payload);
      return;
    }
    query.onUpdate(payload.key, payload.changeType === 'LEAVE' ? null : payload.value);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the routing layer
  private applyQueryResp(query: QueryHandle<any>, payload: QueryRespPayload): void {
    if (payload.error !== undefined || payload.code !== undefined) {
      this.handleQueryError(query, payload.error, payload.code);
      return;
    }

    // A refusal carries no snapshot, so a clean answer in a later chain than the
    // refused one means the server now serves the query. Cleared before the
    // rows apply, so this answer settles the handle for new awaiters.
    const state = this.errorStates.get(query);
    if (state && state.errorKind === 'terminal' && state.errorChain < state.chain) {
      state.errorKind = null;
      query.clearServerError();
    }

    const { results, nextCursor, hasMore, cursorStatus, merkleRootHash } = payload;
    query.onResult(results, 'server', merkleRootHash);
    query.updatePaginationInfo({ nextCursor, hasMore, cursorStatus });
  }

  /** Cancel every pending resubscribe timer. */
  public close(): void {
    for (const query of this.queries.values()) {
      const state = this.errorStates.get(query);
      if (state) this.cancelTimer(state);
    }
  }

  private handleQueryError(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the routing layer
    query: QueryHandle<any>,
    error: string | undefined,
    code: string | undefined,
  ): void {
    const state = this.errorState(query);
    const queryError = new QueryError(
      `${code ?? 'QUERY_FAILED'}: ${error ?? ''}`,
      code !== undefined && KNOWN_QUERY_ERROR_CODES.has(code)
        ? (code as QueryErrorCode)
        : 'QUERY_FAILED',
      query.getMapName(),
    );

    // A one-shot handle builds no delta buffer, so an overflow there is not
    // recoverable by resubscribing; it is terminal like any other code.
    if (code !== SNAPSHOT_OVERFLOW || query.isSnapshotOnly()) {
      this.cancelTimer(state);
      state.errorKind = 'terminal';
      state.errorChain = state.chain;
      query.setServerError(queryError);
      return;
    }

    if (state.overflows >= MAX_OVERFLOW_RESUBSCRIBES) {
      this.cancelTimer(state);
      state.errorKind = 'overflow';
      query.setServerError(queryError);
      return;
    }

    state.overflows += 1;
    if (state.timer !== undefined) return;
    // Full jitter spreads the retries of many clients after a reconnect storm.
    const delay =
      OVERFLOW_RESUBSCRIBE_BASE_MS * 4 ** (state.overflows - 1) * (0.5 + 0.5 * Math.random());
    state.timer = setTimeout(() => {
      state.timer = undefined;
      // Offline: the next resubscribeAll starts a new chain instead.
      if (this.config.isAuthenticated()) {
        this.sendQuerySubscription(query);
      }
    }, delay);
  }

  /**
   * Start a subscription chain: the overflow bound resets and an overflow
   * error clears. A terminal error stays until a clean response in a later
   * chain than the one that raised it clears it.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the routing layer
  private startChain(query: QueryHandle<any>): void {
    const state = this.errorState(query);
    this.cancelTimer(state);
    state.overflows = 0;
    state.chain += 1;
    if (state.errorKind === 'overflow') {
      state.errorKind = null;
      query.clearServerError();
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the routing layer
  private errorState(query: QueryHandle<any>): QueryErrorState {
    let state = this.errorStates.get(query);
    if (!state) {
      state = { chain: 0, overflows: 0, timer: undefined, errorKind: null, errorChain: 0 };
      this.errorStates.set(query, state);
    }
    return state;
  }

  private cancelTimer(state: QueryErrorState): void {
    if (state.timer !== undefined) {
      clearTimeout(state.timer);
      state.timer = undefined;
    }
  }

  // ============================================
  // Hybrid Query Methods
  // ============================================

  /**
   * Subscribe to a hybrid query (FTS + filter combination).
   */
  public subscribeToHybridQuery<T>(query: HybridQueryHandle<T>): void {
    this.hybridQueries.set(query.id, query);

    const filter = query.getFilter();
    const mapName = query.getMapName();

    // If query has FTS predicate and authenticated, send to server
    if (query.hasFTSPredicate() && this.config.isAuthenticated()) {
      this.sendHybridQuerySubscription(query.id, mapName, filter);
    }

    // Load initial local data
    this.runLocalHybridQuery<T>(mapName, filter).then((results) => {
      query.onResult(results, 'local');
    });
  }

  /**
   * Unsubscribe from a hybrid query.
   */
  public unsubscribeFromHybridQuery(queryId: string): void {
    const query = this.hybridQueries.get(queryId);
    if (query) {
      this.hybridQueries.delete(queryId);

      // Notify server to unsubscribe
      if (this.config.isAuthenticated() && query.hasFTSPredicate()) {
        this.config.sendMessage({
          type: 'HYBRID_QUERY_UNSUBSCRIBE',
          payload: { subscriptionId: queryId },
        });
      }
    }
  }

  /**
   * Send hybrid query subscription message to server.
   */
  private sendHybridQuerySubscription(
    queryId: string,
    mapName: string,
    filter: HybridQueryFilter,
  ): void {
    this.config.sendMessage({
      type: 'HYBRID_QUERY_SUBSCRIBE',
      payload: {
        subscriptionId: queryId,
        mapName,
        predicate: filter.predicate,
        where: filter.where,
        sort: filter.sort,
        limit: filter.limit,
        cursor: filter.cursor,
      },
    });
  }

  // ============================================
  // Local Query Execution
  // ============================================

  /**
   * Executes a query against local storage immediately.
   */
  public async runLocalQuery(
    mapName: string,
    filter: QueryFilter,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- local query returns raw record values whose type is unknown at the query manager level; callers cast to T
  ): Promise<{ key: string; value: any }[]> {
    // Fail loudly on clauses we cannot evaluate offline instead of silently
    // returning results that differ from the server.
    this.assertOfflineSupported(filter);

    // Retrieve all keys for the map
    const keys = await this.config.storageAdapter.getAllKeys();
    const mapKeys = keys.filter((k) => k.startsWith(mapName + ':'));

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- accumulator for untyped local records; typed after return by the caller
    const results: { key: string; value: any }[] = [];
    for (const fullKey of mapKeys) {
      const record = await this.config.storageAdapter.get(fullKey);
      if (record && record.value) {
        // Extract actual key from "mapName:key"
        const actualKey = fullKey.slice(mapName.length + 1);

        let matches = true;

        // Apply 'where' (equality)
        if (filter.where) {
          for (const [k, v] of Object.entries(filter.where)) {
            if (record.value[k] !== v) {
              matches = false;
              break;
            }
          }
        }

        // Apply 'predicate'
        if (matches && filter.predicate) {
          if (!evaluatePredicate(filter.predicate, record.value)) {
            matches = false;
          }
        }

        if (matches) {
          results.push({ key: actualKey, value: record.value });
        }
      }
    }

    // Apply the same ordering + windowing the server would, so an offline
    // `query`/`queryOnce({ allowLocal: true })` returns results shaped like the
    // online answer instead of an unsorted, unlimited set.
    if (filter.sort) {
      const sort = filter.sort;
      results.sort((a, b) => this.compareBySort(a, b, sort));
    }
    if (filter.limit) {
      return results.slice(0, filter.limit);
    }
    return results;
  }

  /**
   * Clauses the offline local query engine cannot faithfully evaluate. Rather
   * than silently drop them — returning results that diverge from the server's —
   * reject explicitly so the caller knows the query needs a server connection.
   * (`sort` and `limit` ARE applied offline; only these remain server-only.)
   */
  private assertOfflineSupported(filter: QueryFilter): void {
    const unsupported: string[] = [];
    if (filter.cursor) unsupported.push('cursor');
    if (filter.groupBy && filter.groupBy.length > 0) unsupported.push('groupBy');
    if (filter.aggregations && filter.aggregations.length > 0) unsupported.push('aggregations');
    if (unsupported.length > 0) {
      throw new Error(
        `Offline query does not support ${unsupported.join(', ')}: ` +
          'these clauses require a server connection. Run the query online, or ' +
          'remove them for offline use.',
      );
    }
  }

  /**
   * Sort comparator shared by the offline standard and hybrid local query
   * engines so the two cannot diverge. Honors the `_score` and `_key`
   * pseudo-fields (score defaults to 0 for the non-hybrid path).
   */
  private compareBySort<R extends { key: string; value: unknown; score?: number }>(
    a: R,
    b: R,
    sort: Record<string, 'asc' | 'desc'>,
  ): number {
    for (const [field, direction] of Object.entries(sort)) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic field access by runtime sort field name
      let valA: any;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic field access by runtime sort field name
      let valB: any;
      if (field === '_score') {
        valA = a.score ?? 0;
        valB = b.score ?? 0;
      } else if (field === '_key') {
        valA = a.key;
        valB = b.key;
      } else {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic field access on typed value
        valA = (a.value as any)[field];
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic field access on typed value
        valB = (b.value as any)[field];
      }
      if (valA < valB) return direction === 'asc' ? -1 : 1;
      if (valA > valB) return direction === 'asc' ? 1 : -1;
    }
    return 0;
  }

  /**
   * Run a local hybrid query (FTS + filter combination).
   * For FTS predicates, returns results with score = 0 (local-only mode).
   * Server provides actual FTS scoring.
   */
  public async runLocalHybridQuery<T>(
    mapName: string,
    filter: HybridQueryFilter,
  ): Promise<Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>> {
    const results: Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }> = [];

    // Get all entries from the map using storage adapter
    const allKeys = await this.config.storageAdapter.getAllKeys();
    const mapPrefix = `${mapName}:`;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- raw storage entries before type narrowing; value type determined per-record after deserialization
    const entries: Array<[string, any]> = [];

    for (const fullKey of allKeys) {
      if (fullKey.startsWith(mapPrefix)) {
        const key = fullKey.substring(mapPrefix.length);
        const record = await this.config.storageAdapter.get(fullKey);
        if (record) {
          entries.push([key, record]);
        }
      }
    }

    for (const [key, record] of entries) {
      if (record === null || record.value === null) continue;

      const value = record.value as T;

      // Evaluate predicate (including FTS predicates - basic local evaluation)
      if (filter.predicate) {
        const matches = evaluatePredicate(filter.predicate, value as Record<string, unknown>);
        if (!matches) continue;
      }

      // Evaluate where clause
      if (filter.where) {
        let whereMatches = true;
        for (const [field, expected] of Object.entries(filter.where)) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- value is cast to any for dynamic field access by string key in the where clause
          if ((value as any)[field] !== expected) {
            whereMatches = false;
            break;
          }
        }
        if (!whereMatches) continue;
      }

      results.push({
        key,
        value,
        score: 0, // Local doesn't have FTS scoring
        matchedTerms: [],
      });
    }

    // Sort results (shared comparator with the standard offline path)
    if (filter.sort) {
      const sort = filter.sort;
      results.sort((a, b) => this.compareBySort(a, b, sort));
    }

    // Apply limit (cursor filtering is done server-side)
    let sliced = results;
    if (filter.limit) {
      sliced = sliced.slice(0, filter.limit);
    }

    return sliced;
  }

  // ============================================
  // Re-subscription (after auth)
  // ============================================

  /**
   * Re-subscribe all queries and live searches after authentication.
   * Called by SyncEngine after AUTH_ACK.
   */
  public resubscribeAll(): void {
    logger.debug(
      {
        queryCount: this.queries.size,
        hybridCount: this.hybridQueries.size,
        liveSearchCount: this.liveSearches.size,
      },
      'QueryManager: resubscribing all queries',
    );

    // This runs inside the auth acknowledgement, and a send can throw before it is
    // caught (the payload is serialized outside the send's try): one failing
    // subscription must not stop the others, the topic restore or the CONNECTED
    // transition.
    this.eachLogged(this.queries.values(), (query) => {
      this.startChain(query);
      const wireId = this.sendQuerySubscription(query);

      // Delta reconnect for field-projected queries: send QUERY_SYNC_INIT when we have
      // a stored Merkle root hash so the server can diff and send only changed records
      const filter = query.getFilter();
      if (filter.fields && filter.fields.length > 0 && query.merkleRootHash !== 0) {
        this.config.sendMessage({
          type: 'QUERY_SYNC_INIT',
          payload: {
            queryId: wireId,
            rootHash: query.merkleRootHash,
          },
        });
      }
    });

    // Re-subscribe hybrid queries with FTS predicates
    this.eachLogged(this.hybridQueries.values(), (query) => {
      if (query.hasFTSPredicate()) {
        this.sendHybridQuerySubscription(query.id, query.getMapName(), query.getFilter());
      }
    });

    this.eachLogged(this.liveSearches, (handle) => handle.resubscribe());
  }

  /** Runs `subscribe` for every item; one that throws is logged and the rest go on. */
  private eachLogged<T>(items: Iterable<T>, subscribe: (item: T) => void): void {
    for (const item of items) {
      try {
        subscribe(item);
      } catch (err) {
        logger.error({ err }, 'QueryManager: resubscribe failed');
      }
    }
  }
}
