import { LWWMap, ORMap } from '@topgunbuild/core';
import type {
  ORMapRecord,
  LWWRecord,
  EntryProcessorDef,
  EntryProcessorResult,
  SearchOptions,
} from '@topgunbuild/core';
import type { IStorageAdapter, StorageMutation } from './IStorageAdapter';
import { SyncEngine } from './SyncEngine';
import type { BackoffConfig, SqlQueryResult } from './SyncEngine';
import type {
  VectorSearchClientOptions,
  VectorSearchClientResult,
  HybridSearchClientOptions,
  HybridSearchClientResult,
} from './sync';
import type { AuthProvider } from './auth/types';
import { QueryHandle } from './QueryHandle';
import type { QueryFilter, QueryResultItem } from './QueryHandle';
import {
  QueryOnceUnsettledError,
  QueryOnceLocalError,
  type QueryOnceUnsettledReason,
} from './errors/QueryOnceError';

/**
 * Return value of {@link TopGunClient.queryOncePaged}.
 *
 * `items` is the authoritative server result for this page.
 * `cursor` is the opaque token for the next page (undefined when none).
 * `hasMore` is true when the server signalled additional rows beyond this page.
 */
export interface QueryOncePagedResult<T> {
  items: QueryResultItem<T>[];
  cursor?: string;
  hasMore: boolean;
}
import { DistributedLock } from './DistributedLock';
import { TopicHandle } from './TopicHandle';
import { PNCounterHandle } from './PNCounterHandle';
import { EventJournalReader } from './EventJournalReader';
import { SearchHandle } from './SearchHandle';
import { HybridSearchHandle } from './HybridSearchHandle';
import type { HybridSearchSubscribeOptions } from './HybridSearchHandle';
import { HybridQueryHandle } from './HybridQueryHandle';
import type { HybridQueryFilter } from './HybridQueryHandle';
import { logger } from './utils/logger';
import { assertValidMapName, keyBelongsToLongerHeldName } from './utils/mapName';
import { SyncState } from './SyncState';
import type { StateChangeEvent } from './SyncStateMachine';
import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from './BackpressureConfig';
import { ClusterClient } from './cluster/ClusterClient';
import { SingleServerProvider } from './connection/SingleServerProvider';
import { NullConnectionProvider } from './connection/NullConnectionProvider';
import type { NodeHealth } from '@topgunbuild/core';

// ============================================
// Cluster Configuration Types
// ============================================

/**
 * Cluster mode configuration for TopGunClient.
 * When provided, the client connects to multiple nodes with partition-aware routing.
 */
export interface TopGunClusterConfig {
  /** Initial seed nodes (at least one required) */
  seeds: string[];

  /** Connection pool size per node (default: 1) */
  connectionsPerNode?: number;

  /** Enable smart routing to partition owner (default: true) */
  smartRouting?: boolean;

  /** Partition map refresh interval in ms (default: 30000) */
  partitionMapRefreshMs?: number;

  /** Connection timeout per node in ms (default: 5000) */
  connectionTimeoutMs?: number;

  /** Retry attempts for failed operations (default: 3) */
  retryAttempts?: number;
}

/**
 * Default values for cluster configuration
 */
export const DEFAULT_CLUSTER_CONFIG: Required<Omit<TopGunClusterConfig, 'seeds'>> = {
  connectionsPerNode: 1,
  smartRouting: true,
  partitionMapRefreshMs: 30000,
  connectionTimeoutMs: 5000,
  retryAttempts: 3,
};

/** Default settle-wait timeout for queryOnce, in milliseconds. Non-infinite so a
 * stuck/silent server can never hang the caller indefinitely. */
export const DEFAULT_QUERY_ONCE_TIMEOUT_MS = 5000;

/**
 * Outcome of {@link TopGunClient.confirmWrite}: whether a local write was
 * confirmed applied by the server (`'synced'`) or could not be confirmed —
 * because the client is offline (`'offline'`), the server did not acknowledge in
 * time (`'timeout'`), or the local op could not be recorded (`'failed'`).
 */
export type WriteConfirmation = 'synced' | 'offline' | 'timeout' | 'failed';

/**
 * Options for {@link TopGunClient.queryOnce}, a one-shot read that resolves with
 * authoritative server data (or rejects rather than returning stale local data).
 */
export interface QueryOnceOptions {
  /**
   * Maximum time to wait for the first authoritative server QUERY_RESP, in
   * milliseconds. Defaults to {@link DEFAULT_QUERY_ONCE_TIMEOUT_MS} (5000). There
   * is no infinite default — a silent server can never hang the caller.
   */
  timeoutMs?: number;

  /**
   * When false/unset (default), queryOnce REJECTS (throws QueryOnceUnsettledError)
   * if the client is offline or the settle wait times out — it NEVER silently
   * returns local/stale data.
   *
   * When true, on offline/timeout queryOnce instead throws a typed
   * QueryOnceLocalError carrying the non-settled local snapshot on `.localData`,
   * so the caller can ALWAYS distinguish settled server data (normal resolve)
   * from non-settled local data (typed-error catch).
   */
  allowLocal?: boolean;
}

/**
 * TopGunClient configuration options
 */
export interface TopGunClientConfig {
  /** Unique node identifier (auto-generated if not provided) */
  nodeId?: string;

  /** Single-server mode: WebSocket URL to connect to */
  serverUrl?: string;

  /** Cluster mode: Configuration for multi-node routing */
  cluster?: TopGunClusterConfig;

  /** Storage adapter for local persistence */
  storage: IStorageAdapter;

  /** Backoff configuration for reconnection */
  backoff?: Partial<BackoffConfig>;

  /** Backpressure configuration */
  backpressure?: Partial<BackpressureConfig>;

  /** Auth provider for automatic token management */
  auth?: AuthProvider;

  /**
   * Called when the server demands authentication (sends AUTH_REQUIRED) but no
   * token / token provider / auth provider is configured. Without this hook the
   * client parks silently in AUTHENTICATING. Wire it to prompt for login, call
   * setAuthToken(), redirect, etc.
   */
  onAuthRequired?: (error: import('./errors/AuthRequiredError').AuthRequiredError) => void;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- TSchema defaults to any so untyped callers work without a schema; narrowed via generic at instantiation
export class TopGunClient<TSchema extends Record<string, any> = any> {
  private readonly nodeId: string;
  private readonly syncEngine: SyncEngine;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- maps registry holds heterogeneous LWWMap and ORMap instances; value types differ per map name and are narrowed at getMap/getORMap call sites
  private readonly maps: Map<string, LWWMap<any, any> | ORMap<any, any>> = new Map();
  private readonly storageAdapter: IStorageAdapter;
  private readonly topicHandles: Map<string, TopicHandle> = new Map();
  private readonly counters: Map<string, PNCounterHandle> = new Map();
  private readonly clusterClient?: ClusterClient;
  private readonly isClusterMode: boolean;
  private readonly clusterConfig?: Required<Omit<TopGunClusterConfig, 'seeds'>> & {
    seeds: string[];
  };
  private readonly authProvider?: AuthProvider;

  /**
   * The single-server WebSocket URL this client was configured with, retained so
   * callers (e.g. the MCP server's map-enumeration tool) can derive the matching
   * HTTP control-plane base. `undefined` in cluster or local-only mode, where no
   * single authoritative URL exists.
   */
  private readonly serverUrl?: string;

  /**
   * Most-recent in-flight op promise per `${map}:${key}`, set synchronously by the
   * getMap set/remove wrappers. {@link confirmWrite} awaits the op id from this
   * promise, then waits for the server ack — letting a write be reported as durable
   * only once the server has actually applied it. Entries are deleted once
   * confirmed. Keyed by the latest write to a (map,key); serial callers (the MCP
   * mutate tool) never overlap two writes to one key.
   */
  private readonly inFlightWrites: Map<string, Promise<string>> = new Map();

  constructor(config: TopGunClientConfig) {
    // Supplying both serverUrl and cluster is ambiguous — fail early
    if (config.serverUrl && config.cluster) {
      throw new Error('Cannot specify both serverUrl and cluster config');
    }

    this.nodeId = config.nodeId || crypto.randomUUID();
    this.storageAdapter = config.storage;
    this.isClusterMode = !!config.cluster;

    if (config.cluster) {
      // Validate cluster seeds
      if (!config.cluster.seeds || config.cluster.seeds.length === 0) {
        throw new Error('Cluster config requires at least one seed node');
      }

      // Merge with defaults
      this.clusterConfig = {
        seeds: config.cluster.seeds,
        connectionsPerNode:
          config.cluster.connectionsPerNode ?? DEFAULT_CLUSTER_CONFIG.connectionsPerNode,
        smartRouting: config.cluster.smartRouting ?? DEFAULT_CLUSTER_CONFIG.smartRouting,
        partitionMapRefreshMs:
          config.cluster.partitionMapRefreshMs ?? DEFAULT_CLUSTER_CONFIG.partitionMapRefreshMs,
        connectionTimeoutMs:
          config.cluster.connectionTimeoutMs ?? DEFAULT_CLUSTER_CONFIG.connectionTimeoutMs,
        retryAttempts: config.cluster.retryAttempts ?? DEFAULT_CLUSTER_CONFIG.retryAttempts,
      };

      // Initialize cluster mode
      this.clusterClient = new ClusterClient({
        enabled: true,
        seedNodes: this.clusterConfig.seeds,
        routingMode: this.clusterConfig.smartRouting ? 'direct' : 'forward',
        connectionPool: {
          maxConnectionsPerNode: this.clusterConfig.connectionsPerNode,
          connectionTimeoutMs: this.clusterConfig.connectionTimeoutMs,
        },
        routing: {
          mapRefreshIntervalMs: this.clusterConfig.partitionMapRefreshMs,
        },
      });

      // SyncEngine uses ClusterClient as connectionProvider for partition-aware routing
      this.syncEngine = new SyncEngine({
        nodeId: this.nodeId,
        connectionProvider: this.clusterClient,
        storageAdapter: this.storageAdapter,
        backoff: config.backoff,
        backpressure: config.backpressure,
        onAuthRequired: config.onAuthRequired,
      });

      logger.info({ seeds: this.clusterConfig.seeds }, 'TopGunClient initialized in cluster mode');
    } else if (config.serverUrl) {
      // Single-server mode: create SingleServerProvider from serverUrl
      // Retain the URL so getServerUrl() can expose it without reaching into the
      // connection provider.
      this.serverUrl = config.serverUrl;
      // Map BackoffConfig to SingleServerProviderConfig for unified retry behavior
      const singleServerProvider = new SingleServerProvider({
        url: config.serverUrl,
        maxReconnectAttempts: config.backoff?.maxRetries,
        reconnectDelayMs: config.backoff?.initialDelayMs,
        backoffMultiplier: config.backoff?.multiplier,
        maxReconnectDelayMs: config.backoff?.maxDelayMs,
      });

      this.syncEngine = new SyncEngine({
        nodeId: this.nodeId,
        connectionProvider: singleServerProvider,
        storageAdapter: this.storageAdapter,
        backoff: config.backoff,
        backpressure: config.backpressure,
        onAuthRequired: config.onAuthRequired,
      });

      logger.info(
        { serverUrl: config.serverUrl },
        'TopGunClient initialized in single-server mode',
      );
    } else {
      // Local-only mode: no sync target — use NullConnectionProvider so SyncEngine
      // is wired correctly but never opens a socket or enters reconnect loops
      const nullProvider = new NullConnectionProvider();
      this.syncEngine = new SyncEngine({
        nodeId: this.nodeId,
        connectionProvider: nullProvider,
        storageAdapter: this.storageAdapter,
        backoff: config.backoff,
        backpressure: config.backpressure,
        onAuthRequired: config.onAuthRequired,
      });
      logger.info({}, 'TopGunClient initialized in local-only mode (no sync target)');
    }

    // Wire auth provider if supplied
    if (config.auth) {
      this.authProvider = config.auth;
      this.authProvider.initialize?.();
      this.syncEngine.setTokenProvider(() => config.auth!.getToken());
    }
  }

  public async start(): Promise<void> {
    await this.storageAdapter.initialize('topgun_offline_db');
    // this.syncEngine.start();
  }

  public setAuthToken(token: string): void {
    this.syncEngine.setAuthToken(token);
  }

  public setAuthTokenProvider(provider: () => Promise<string | null>): void {
    this.syncEngine.setTokenProvider(provider);
  }

  /**
   * Creates a live query subscription for a map.
   *
   * When TSchema is concrete, passing a schema key narrows the QueryHandle
   * value type to TSchema[K]. The untyped overload preserves back-compat.
   */
  public query<K extends keyof TSchema & string>(
    mapName: K,
    filter: QueryFilter,
  ): QueryHandle<TSchema[K]>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- untyped overload for back-compat callers that do not supply a schema type parameter
  public query<T = any>(mapName: string, filter: QueryFilter): QueryHandle<T>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- implementation signature uses any to satisfy both overloads; return type is narrowed by the overload the caller selects
  public query(mapName: string, filter: QueryFilter): QueryHandle<any> {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- QueryHandle constructed for untyped internal storage; actual type flows from the selected overload
    return new QueryHandle<any>(this.syncEngine, mapName, filter);
  }

  /**
   * One-shot read: resolves with the AUTHORITATIVE server result for a query,
   * then auto-unsubscribes (no live subscription leaks). This is the InstantDB-
   * style "give me the truth once" primitive that AI agents and request/response
   * handlers want — unlike {@link query}, it never leaves a live QueryHandle open.
   *
   * Offline policy (explicit, never silently stale):
   * - Default ({@link QueryOnceOptions.allowLocal} unset/false): if the client is
   *   offline OR the settle wait times out, REJECTS with {@link QueryOnceUnsettledError}.
   * - `{ allowLocal: true }`: on offline/timeout, throws a typed
   *   {@link QueryOnceLocalError} carrying the non-settled local snapshot on
   *   `.localData`. We chose a typed-error fallback (not a `{ settled, data }`
   *   wrapper return) so the happy path stays a plain `Promise<items[]>` and a
   *   normal resolve is ALWAYS settled server data while a caught QueryOnceLocalError
   *   is ALWAYS non-settled local data — there is no in-band ambiguity to inspect.
   */
  public queryOnce<K extends keyof TSchema & string>(
    mapName: K,
    filter: QueryFilter,
    opts?: QueryOnceOptions,
  ): Promise<QueryResultItem<TSchema[K]>[]>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- untyped overload for back-compat callers that do not supply a schema type parameter
  public queryOnce<T = any>(
    mapName: string,
    filter: QueryFilter,
    opts?: QueryOnceOptions,
  ): Promise<QueryResultItem<T>[]>;
  public async queryOnce(
    mapName: string,
    filter: QueryFilter,
    opts?: QueryOnceOptions,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- implementation signature uses any to satisfy both overloads; return type is narrowed by the overload the caller selects
  ): Promise<QueryResultItem<any>[]> {
    const timeoutMs = opts?.timeoutMs ?? DEFAULT_QUERY_ONCE_TIMEOUT_MS;
    const allowLocal = opts?.allowLocal ?? false;

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- handle storage is untyped internally; result type flows from the selected overload
    const handle = new QueryHandle<any>(this.syncEngine, mapName, filter);

    // subscribe() both activates the server subscription (first listener triggers
    // subscribeToQuery) and feeds us the latest sorted results; we keep the most
    // recent snapshot so we can read it after settlement without a private accessor.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- latest sorted snapshot for the active query; element type narrowed by the selected overload
    let latest: QueryResultItem<any>[] = [];
    const unsubscribe = handle.subscribe((results) => {
      latest = results;
    });

    const cleanup = (): void => {
      // Auto-unsubscribe so queryOnce never leaks a live subscription.
      unsubscribe();
    };

    try {
      // Reject up front when offline (unless allowing local) — no point waiting for
      // a server response that cannot arrive. Use the PUBLIC connection state.
      if (!this.isClientOnline()) {
        // When allowing local, let the handle's async local pre-load settle into
        // `latest` first so the returned snapshot is populated, not empty.
        if (allowLocal) {
          await this.flushLocalPreload();
        }
        return this.handleUnsettled('offline', mapName, latest, allowLocal);
      }

      const settled = await this.raceSettle(handle, timeoutMs);
      if (!settled) {
        return this.handleUnsettled('timeout', mapName, latest, allowLocal);
      }

      // Settled: `latest` reflects the authoritative server result (even if empty).
      return latest;
    } finally {
      cleanup();
    }
  }

  /**
   * One-shot paged read: resolves with authoritative server data including cursor
   * metadata for pagination. Unlike {@link queryOnce}, the return value carries
   * `{ items, cursor, hasMore }` so callers can drive {@link QueryHandle.loadMore}
   * or issue a follow-up `queryOncePaged` for the next page.
   *
   * The offline policy is identical to {@link queryOnce}:
   * - Default: rejects with {@link QueryOnceUnsettledError} when offline or timed-out.
   * - `{ allowLocal: true }`: throws {@link QueryOnceLocalError} carrying a local
   *   snapshot on `.localData`.
   *
   * `queryOnce` is left unchanged — this is a separate method so that the plain
   * `Promise<items[]>` return type of `queryOnce` is not disturbed.
   */
  public queryOncePaged<K extends keyof TSchema & string>(
    mapName: K,
    filter: QueryFilter,
    opts?: QueryOnceOptions,
  ): Promise<QueryOncePagedResult<TSchema[K]>>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- untyped overload for back-compat callers that do not supply a schema type parameter
  public queryOncePaged<T = any>(
    mapName: string,
    filter: QueryFilter,
    opts?: QueryOnceOptions,
  ): Promise<QueryOncePagedResult<T>>;
  public async queryOncePaged(
    mapName: string,
    filter: QueryFilter,
    opts?: QueryOnceOptions,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- implementation signature uses any to satisfy both overloads; return type is narrowed by the overload the caller selects
  ): Promise<QueryOncePagedResult<any>> {
    const timeoutMs = opts?.timeoutMs ?? DEFAULT_QUERY_ONCE_TIMEOUT_MS;
    const allowLocal = opts?.allowLocal ?? false;

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- handle storage is untyped internally; result type flows from the selected overload
    const handle = new QueryHandle<any>(this.syncEngine, mapName, filter);

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- latest sorted snapshot for the active query; element type narrowed by the selected overload
    let latest: QueryResultItem<any>[] = [];
    const unsubscribe = handle.subscribe((results) => {
      latest = results;
    });

    const cleanup = (): void => {
      unsubscribe();
    };

    try {
      if (!this.isClientOnline()) {
        if (allowLocal) {
          await this.flushLocalPreload();
        }
        return this.handleUnsettledPaged('offline', mapName, latest, allowLocal, {
          hasMore: false,
        });
      }

      const settled = await this.raceSettle(handle, timeoutMs);
      if (!settled) {
        return this.handleUnsettledPaged('timeout', mapName, latest, allowLocal, {
          hasMore: false,
        });
      }

      const paginationInfo = handle.getPaginationInfo();
      return {
        items: latest,
        cursor: paginationInfo.nextCursor,
        hasMore: paginationInfo.hasMore,
      };
    } finally {
      cleanup();
    }
  }

  /**
   * Resolves true when the query settles (first server QUERY_RESP), false when the
   * timeout fires first. Clears the timer on settlement so it never dangles.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- handle is the untyped internal QueryHandle; settlement is value-agnostic
  private raceSettle(handle: QueryHandle<any>, timeoutMs: number): Promise<boolean> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<boolean>((resolve) => {
      timer = setTimeout(() => resolve(false), timeoutMs);
    });
    const settled = handle.whenSettled().then(() => true);
    return Promise.race([settled, timeout]).finally(() => {
      if (timer !== undefined) clearTimeout(timer);
    });
  }

  /**
   * Build the offline/timeout outcome: throw the hard-reject error by default, or
   * throw the typed local-fallback error carrying the snapshot when allowLocal.
   */
  private handleUnsettled(
    reason: QueryOnceUnsettledReason,
    mapName: string,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- local snapshot element type flows from the selected overload at the call site
    localData: QueryResultItem<any>[],
    allowLocal: boolean,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- never actually returns; typed to satisfy the queryOnce return contract
  ): Promise<QueryResultItem<any>[]> {
    if (allowLocal) {
      throw new QueryOnceLocalError(reason, mapName, localData);
    }
    throw new QueryOnceUnsettledError(reason, mapName);
  }

  /**
   * Build the offline/timeout outcome for queryOncePaged.
   * Mirrors handleUnsettled but returns QueryOncePagedResult on allowLocal.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- never actually returns; typed to satisfy the queryOncePaged return contract
  private handleUnsettledPaged(
    reason: QueryOnceUnsettledReason,
    mapName: string,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- local snapshot element type flows from the selected overload at the call site
    localData: QueryResultItem<any>[],
    allowLocal: boolean,
    paginationFallback: { hasMore: boolean; cursor?: string },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- never actually returns; typed to satisfy the queryOncePaged return contract
  ): Promise<QueryOncePagedResult<any>> {
    if (allowLocal) {
      throw new QueryOnceLocalError(reason, mapName, localData);
    }
    throw new QueryOnceUnsettledError(reason, mapName);
    // paginationFallback would only be used in a non-throwing path; included for
    // future extensibility.
    void paginationFallback;
  }

  /**
   * Yield the microtask queue so the QueryHandle's async local pre-load
   * (loadInitialLocalData → onResult('local')) lands in our captured snapshot
   * before we build an allowLocal fallback. Two yields cover the promise + its
   * .then continuation. Bounded and deterministic — no timers.
   */
  private async flushLocalPreload(): Promise<void> {
    await Promise.resolve();
    await Promise.resolve();
  }

  /**
   * Whether the client currently has (or is establishing) a live server connection.
   * Reads the PUBLIC SyncEngine connection state — does not widen any private flag.
   */
  private isClientOnline(): boolean {
    const state = this.syncEngine.getConnectionState();
    return (
      state === SyncState.CONNECTING ||
      state === SyncState.AUTHENTICATING ||
      state === SyncState.SYNCING ||
      state === SyncState.CONNECTED
    );
  }

  /**
   * Retrieves a distributed lock instance.
   * @param name The name of the lock.
   */
  public getLock(name: string): DistributedLock {
    return new DistributedLock(this.syncEngine, name);
  }

  /**
   * Retrieves a topic handle for Pub/Sub messaging.
   * @param name The name of the topic.
   */
  public topic(name: string): TopicHandle {
    if (!this.topicHandles.has(name)) {
      this.topicHandles.set(name, new TopicHandle(this.syncEngine, name));
    }
    return this.topicHandles.get(name)!;
  }

  /**
   * Retrieves a PN Counter instance. If the counter doesn't exist locally, it's created.
   * PN Counters support increment and decrement operations that work offline
   * and sync to server when connected.
   *
   * @param name The name of the counter (e.g., 'likes:post-123')
   * @returns A PNCounterHandle instance
   *
   * @example
   * ```typescript
   * const likes = client.getPNCounter('likes:post-123');
   * likes.increment(); // +1
   * likes.decrement(); // -1
   * likes.addAndGet(10); // +10
   *
   * likes.subscribe((value) => {
   *   console.log('Current likes:', value);
   * });
   * ```
   */
  public getPNCounter(name: string): PNCounterHandle {
    let counter = this.counters.get(name);
    if (!counter) {
      counter = new PNCounterHandle(name, this.nodeId, this.syncEngine, this.storageAdapter);
      this.counters.set(name, counter);
    }
    return counter;
  }

  /**
   * Retrieves an LWWMap instance. If the map doesn't exist locally, it's created.
   *
   * When TSchema is concrete, passing a key that exists in the schema narrows
   * the return type to LWWMap<string, TSchema[K]>. The untyped overload
   * preserves back-compat for callers that supply explicit type parameters.
   *
   * @param name The name of the map.
   * @returns An LWWMap instance.
   */
  public getMap<K extends keyof TSchema & string>(name: K): LWWMap<string, TSchema[K]>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- untyped overload for back-compat callers that do not supply a schema type parameter
  public getMap<K = string, V = any>(name: string): LWWMap<K, V>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- implementation signature uses any to satisfy both overloads; actual type flows from the selected overload
  public getMap(name: string): LWWMap<any, any> {
    // Reject an invalid name BEFORE any registry/restore side effect so a
    // rejected name leaves this.maps and the sync registry untouched.
    assertValidMapName(name);
    if (this.maps.has(name)) {
      const map = this.maps.get(name);
      if (map instanceof LWWMap) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- cast to satisfy overload; actual type narrowed by caller
        return map as LWWMap<any, any>;
      }
      throw new Error(`Map ${name} exists but is not an LWWMap`);
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- LWWMap generic params erased at the registry level; actual type provided by the caller's overload
    const lwwMap = new LWWMap<any, any>(this.syncEngine.getHLC());
    this.maps.set(name, lwwMap);
    this.syncEngine.registerMap(name, lwwMap);

    // Restore state from storage asynchronously
    this.storageAdapter
      .getAllKeys()
      .then(async (keys) => {
        const mapPrefix = `${name}:`;
        for (const fullKey of keys) {
          if (fullKey.startsWith(mapPrefix)) {
            const record = await this.storageAdapter.get(fullKey);
            // eslint-disable-next-line @typescript-eslint/no-explicit-any -- record shape is unknown at restore time; timestamp check distinguishes LWWRecord from ORMapRecord without importing the schema
            if (record && (record as LWWRecord<any>).timestamp && !(record as any).tag) {
              // Strip prefix to get actual key
              // eslint-disable-next-line @typescript-eslint/no-explicit-any -- key is a string that needs to be cast back to the map's K type; the caller's overload constrains K at the public API
              const key = fullKey.substring(mapPrefix.length) as unknown as any;
              // Merge into in-memory map without triggering new ops
              // eslint-disable-next-line @typescript-eslint/no-explicit-any -- LWWRecord<any> used for the merge; actual value type erased at restore time and re-typed by the public overload
              lwwMap.merge(key, record as LWWRecord<any>);
            }
          }
        }
      })
      .catch((err) => logger.error({ err }, 'Failed to restore keys from storage'));

    // Wrap LWWMap with IMap interface logic
    const originalSet = lwwMap.set.bind(lwwMap);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- key and value types are erased at the wrapper level; actual types flow from the caller's overload
    lwwMap.set = (key: any, value: any, ttlMs?: number) => {
      const record = originalSet(key, value, ttlMs);
      // Atomic durable write: the KV record + its op-log entry commit in ONE transaction
      // (crash-consistent — no record-without-op / op-without-record). The optimistic
      // in-memory mutation above is unaffected; on commit failure the op is not queued.
      const mutations: StorageMutation[] = [
        { store: 'kv', type: 'put', key: `${name}:${key}`, value: record },
      ];
      const opPromise = this.syncEngine.recordOperation(
        name,
        'PUT',
        String(key),
        { record, timestamp: record.timestamp },
        mutations,
      );
      this.trackInFlightWrite(name, String(key), opPromise, 'PUT');
      return record;
    };

    const originalRemove = lwwMap.remove.bind(lwwMap);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- key type is erased at the wrapper level; actual type flows from the caller's overload
    lwwMap.remove = (key: any) => {
      const tombstone = originalRemove(key);
      const mutations: StorageMutation[] = [
        { store: 'kv', type: 'put', key: `${name}:${key}`, value: tombstone },
      ];
      const opPromise = this.syncEngine.recordOperation(
        name,
        'REMOVE',
        String(key),
        { record: tombstone, timestamp: tombstone.timestamp },
        mutations,
      );
      this.trackInFlightWrite(name, String(key), opPromise, 'REMOVE');
      return tombstone;
    };

    return lwwMap;
  }

  /**
   * Retrieves an ORMap instance. If the map doesn't exist locally, it's created.
   *
   * When TSchema is concrete, the return type narrows to ORMap<string, TSchema[K]>.
   * The untyped overload preserves back-compat for explicit type-parameter callers.
   *
   * @param name The name of the map.
   * @returns An ORMap instance.
   */
  public getORMap<K extends keyof TSchema & string>(name: K): ORMap<string, TSchema[K]>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- untyped overload for back-compat callers that do not supply a schema type parameter
  public getORMap<K = string, V = any>(name: string): ORMap<K, V>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- implementation signature uses any to satisfy both overloads; actual type flows from the selected overload
  public getORMap(name: string): ORMap<any, any> {
    // Reject an invalid name BEFORE any registry/restore side effect so a
    // rejected name leaves this.maps and the sync registry untouched.
    assertValidMapName(name);
    if (this.maps.has(name)) {
      const map = this.maps.get(name);
      if (map instanceof ORMap) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- cast to satisfy overload; actual type narrowed by caller
        return map as ORMap<any, any>;
      }
      throw new Error(`Map ${name} exists but is not an ORMap`);
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap generic params erased at the registry level; actual type provided by the caller's overload
    const orMap = new ORMap<any, any>(this.syncEngine.getHLC());
    this.maps.set(name, orMap);
    this.syncEngine.registerMap(name, orMap);

    // Restore state from storage
    this.restoreORMap(name, orMap);

    // Wrap ORMap methods to record operations
    const originalAdd = orMap.add.bind(orMap);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- key and value types are erased at the wrapper level; actual types flow from the caller's overload
    orMap.add = (key: any, value: any, ttlMs?: number) => {
      const record = originalAdd(key, value, ttlMs);

      // Atomic durable write: the records-array KV put + its op-log entry in ONE transaction.
      const mutations: StorageMutation[] = [
        { store: 'kv', type: 'put', key: `${name}:${key}`, value: orMap.getRecords(key) },
      ];
      this.syncEngine
        .recordOperation(
          name,
          'OR_ADD',
          String(key),
          { orRecord: record, timestamp: record.timestamp },
          mutations,
        )
        .catch((err) => logger.error({ err }, 'Failed to commit OR_ADD op'));
      return record;
    };

    const originalRemove = orMap.remove.bind(orMap);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- key and value types are erased at the wrapper level; actual types flow from the caller's overload
    orMap.remove = (key: any, value: any) => {
      const tombstones = originalRemove(key, value);
      const timestamp = this.syncEngine.getHLC().now();

      // The removed key's record list shrank and the tombstone set grew — commit both the KV
      // records-array (or delete when empty) and the tombstone meta atomically with the FIRST
      // OR_REMOVE op. Subsequent tombstone tags for this remove are op-only appends; the
      // durable KV/meta state is already captured by the first commit.
      const records = orMap.getRecords(key);
      const mutations: StorageMutation[] = [
        {
          store: 'kv',
          type: records.length > 0 ? 'put' : 'remove',
          key: `${name}:${key}`,
          value: records,
        },
        {
          store: 'meta',
          type: 'put',
          key: `__sys__:${name}:tombstones`,
          value: orMap.getTombstones(),
        },
      ];

      let first = true;
      for (const tag of tombstones) {
        this.syncEngine
          .recordOperation(
            name,
            'OR_REMOVE',
            String(key),
            { orTag: tag, timestamp },
            first ? mutations : undefined,
          )
          .catch((err) => logger.error({ err }, 'Failed to commit OR_REMOVE op'));
        first = false;
      }
      return tombstones;
    };

    return orMap;
  }

  private async restoreORMap<K, V>(name: string, orMap: ORMap<K, V>) {
    try {
      // 1. Restore Tombstones
      const tombstoneKey = `__sys__:${name}:tombstones`;
      const tombstones = await this.storageAdapter.getMeta(tombstoneKey);
      if (Array.isArray(tombstones)) {
        for (const tag of tombstones) {
          orMap.applyTombstone(tag);
        }
      }

      // 2. Restore Items
      const keys = await this.storageAdapter.getAllKeys();
      const mapPrefix = `${name}:`;
      // Longest-held-name discriminator: the flat key scheme is not injective
      // for legacy colon-named stores, so a key matched by prefix `name:` may
      // actually belong to a LONGER held name (e.g. `a:b:k` belongs to `a:b`,
      // not `a`). Share the held-set from the SyncEngine so this seam and
      // instantiateAndRestoreOrMap skip the same keys.
      const heldNames = this.syncEngine.getHeldOrMapNames();
      for (const fullKey of keys) {
        if (fullKey.startsWith(mapPrefix)) {
          const keyPart = fullKey.substring(mapPrefix.length);
          if (keyBelongsToLongerHeldName(name, keyPart, heldNames)) {
            continue;
          }

          const data = await this.storageAdapter.get(fullKey);
          if (Array.isArray(data)) {
            // It's likely an ORMap value list (Array of ORMapRecord)
            const records = data as ORMapRecord<V>[];
            const key = keyPart as unknown as K;

            for (const record of records) {
              orMap.apply(key, record);
            }
          }
        }
      }
    } catch (e) {
      logger.error({ mapName: name, err: e }, 'Failed to restore ORMap');
    }
  }

  /**
   * Closes the client, disconnecting from the server and cleaning up resources.
   * Returns a Promise so callers can await full teardown, including cluster
   * reconnect timers that would otherwise outlive the close() call and leave
   * dangling setTimeout handles in the process.
   */
  public async close(): Promise<void> {
    this.authProvider?.destroy?.();
    if (this.clusterClient) {
      // Await cluster teardown so reconnect timers are cleared before the method
      // returns — otherwise the process may keep dangling setTimeout handles alive
      // from the WebSocket onclose → scheduleReconnect race that fires after
      // close() returns.
      await this.clusterClient.close();
    }
    this.syncEngine.close();
    // WebSocketManager.close() fires provider.close() without await (void return).
    // Awaiting the provider directly here ensures all reconnect timers are cleared
    // before close() returns — otherwise pending connectionTimeoutId / reconnectTimer
    // handles keep the Jest event loop alive after every test that creates a client.
    await this.syncEngine
      .getConnectionProvider()
      .close()
      .catch(() => {
        // Error already logged inside provider.close()
      });
  }

  // ============================================
  // Cluster Mode API
  // ============================================

  /**
   * Check if running in cluster mode
   */
  public isCluster(): boolean {
    return this.isClusterMode;
  }

  /**
   * Get list of connected cluster nodes (cluster mode only)
   * @returns Array of connected node IDs, or empty array in single-server mode
   */
  public getConnectedNodes(): string[] {
    if (!this.clusterClient) return [];
    return this.clusterClient.getConnectedNodes();
  }

  /**
   * Get the current partition map version (cluster mode only)
   * @returns Partition map version, or 0 in single-server mode
   */
  public getPartitionMapVersion(): number {
    if (!this.clusterClient) return 0;
    return this.clusterClient.getRouterStats().mapVersion;
  }

  /**
   * Check if direct routing is active (cluster mode only)
   * Direct routing sends operations directly to partition owners.
   * @returns true if routing is active, false otherwise
   */
  public isRoutingActive(): boolean {
    if (!this.clusterClient) return false;
    return this.clusterClient.isRoutingActive();
  }

  /**
   * Get health status for all cluster nodes (cluster mode only)
   * @returns Map of node IDs to their health status
   */
  public getClusterHealth(): Map<string, NodeHealth> {
    if (!this.clusterClient) return new Map();
    return this.clusterClient.getHealthStatus();
  }

  /**
   * Force refresh of partition map (cluster mode only)
   * Use this after detecting routing errors.
   */
  public async refreshPartitionMap(): Promise<void> {
    if (!this.clusterClient) return;
    await this.clusterClient.refreshPartitionMap();
  }

  /**
   * Get cluster router statistics (cluster mode only)
   */
  public getClusterStats(): {
    mapVersion: number;
    partitionCount: number;
    nodeCount: number;
    lastRefresh: number;
    isStale: boolean;
  } | null {
    if (!this.clusterClient) return null;
    return this.clusterClient.getRouterStats();
  }

  // ============================================
  // Connection State API
  // ============================================

  /**
   * Get the current connection state
   */
  public getConnectionState(): SyncState {
    return this.syncEngine.getConnectionState();
  }

  /**
   * The single-server WebSocket URL this client was configured with, or
   * `undefined` in cluster / local-only mode. Exposed so callers that need to
   * reach the server's HTTP control plane (e.g. the MCP map-enumeration tool)
   * can derive the matching base URL rather than guessing a default.
   */
  public getServerUrl(): string | undefined {
    return this.serverUrl;
  }

  /**
   * Resolve the auth token currently used to authenticate with the server,
   * whether it was set directly via {@link setAuthToken} or supplied by a token
   * provider (config.auth / {@link setAuthTokenProvider}). Returns `null` when no
   * credentials are configured. Exposed for callers that must authenticate a
   * side-channel request to the server (e.g. the MCP map-enumeration tool calling
   * the HTTP control plane).
   */
  public getAuthToken(): Promise<string | null> {
    return this.syncEngine.getResolvedToken();
  }

  /**
   * Subscribe to connection state changes
   * @param listener Callback function called on each state change
   * @returns Unsubscribe function
   */
  public onConnectionStateChange(listener: (event: StateChangeEvent) => void): () => void {
    return this.syncEngine.onConnectionStateChange(listener);
  }

  /**
   * Get state machine history for debugging
   * @param limit Maximum number of entries to return
   */
  public getStateHistory(limit?: number): StateChangeEvent[] {
    return this.syncEngine.getStateHistory(limit);
  }

  /**
   * Reset the connection and state machine.
   * Use after fatal errors to start fresh.
   */
  public resetConnection(): void {
    this.syncEngine.resetConnection();
  }

  // ============================================
  // Write confirmation API
  // ============================================

  /**
   * Record the in-flight op promise for the latest write to a (map, key), so
   * {@link confirmWrite} can await its server ack. Attaches a `.catch` so an
   * unawaited commit failure never becomes an unhandled rejection, and clears the
   * entry once the op id resolves or rejects (the ack itself is then awaited via
   * the op id, independent of this map).
   */
  private trackInFlightWrite(
    name: string,
    key: string,
    opPromise: Promise<string>,
    opType: 'PUT' | 'REMOVE',
  ): void {
    const writeKey = `${name}:${key}`;
    this.inFlightWrites.set(writeKey, opPromise);
    opPromise.catch((err) => logger.error({ err, name, key }, `Failed to commit ${opType} op`));
  }

  /**
   * Wait until the latest local write to `(map, key)` has been confirmed applied
   * by the server, or report that it could not be confirmed.
   *
   * - `'synced'` — the server acknowledged the write; it is durable server-side.
   * - `'offline'` — the client is not connected; the write is queued locally and
   *   will sync on reconnect, but is NOT yet durable on the server.
   * - `'timeout'` — connected, but the server did not acknowledge within
   *   `timeoutMs`; the write is not yet confirmed durable.
   * - `'failed'` — there was no recordable write to confirm (no tracked op, or
   *   the local op could not be committed).
   *
   * This is the honest "did the server take my write?" answer that callers
   * mutating a database (e.g. the MCP `mutate` tool) need before reporting
   * success — never an optimistic local-only echo.
   */
  public async confirmWrite(
    name: string,
    key: string,
    timeoutMs = 5000,
  ): Promise<WriteConfirmation> {
    const writeKey = `${name}:${key}`;
    const opPromise = this.inFlightWrites.get(writeKey);
    // No tracked write ⇒ we have nothing to confirm. NEVER assume success here:
    // returning 'synced' for an unknown write would violate the core contract
    // (report only server-confirmed state). Callers always write before
    // confirming, so this only fires on misuse — fail closed, not open.
    if (!opPromise) return 'failed';

    let opId: string;
    try {
      opId = await opPromise;
    } catch {
      if (this.inFlightWrites.get(writeKey) === opPromise) {
        this.inFlightWrites.delete(writeKey);
      }
      return 'failed';
    }

    const outcome = await this.syncEngine.waitForOpSynced(opId, timeoutMs);
    // Forget the in-flight write ONLY once the server has confirmed it. On
    // offline/timeout we keep the entry so a later retry re-waits on the same op
    // instead of hitting the no-tracked-write path above and reporting a false
    // result. A subsequent write to the same key overwrites the entry, so this
    // never grows unbounded.
    if (outcome === 'synced' && this.inFlightWrites.get(writeKey) === opPromise) {
      this.inFlightWrites.delete(writeKey);
    }
    return outcome;
  }

  // ============================================
  // Backpressure API
  // ============================================

  /**
   * Get the current number of pending (unacknowledged) operations.
   */
  public getPendingOpsCount(): number {
    return this.syncEngine.getPendingOpsCount();
  }

  /**
   * Get the current backpressure status.
   */
  public getBackpressureStatus(): BackpressureStatus {
    return this.syncEngine.getBackpressureStatus();
  }

  /**
   * Returns true if writes are currently paused due to backpressure.
   */
  public isBackpressurePaused(): boolean {
    return this.syncEngine.isBackpressurePaused();
  }

  /**
   * Subscribe to backpressure events.
   *
   * Available events:
   * - 'backpressure:high': Emitted when pending ops reach high water mark
   * - 'backpressure:low': Emitted when pending ops drop below low water mark
   * - 'backpressure:paused': Emitted when writes are paused (pause strategy)
   * - 'backpressure:resumed': Emitted when writes resume after being paused
   * - 'operation:dropped': Emitted when an operation is dropped (drop-oldest strategy)
   *
   * The listener type narrows by event name: threshold events carry
   * `{ pending, max }`, `'operation:dropped'` carries the dropped-op descriptor,
   * and the paused/resumed transitions carry no payload.
   *
   * @param event Event name
   * @param listener Callback function
   * @returns Unsubscribe function
   *
   * @example
   * ```typescript
   * client.onBackpressure('backpressure:high', ({ pending, max }) => {
   *   console.warn(`Warning: ${pending}/${max} pending ops`);
   * });
   *
   * client.onBackpressure('operation:dropped', ({ mapName, key }) => {
   *   console.warn(`Dropped ${mapName}:${key} under backpressure`);
   * });
   *
   * client.onBackpressure('backpressure:paused', () => {
   *   showLoadingSpinner();
   * });
   *
   * client.onBackpressure('backpressure:resumed', () => {
   *   hideLoadingSpinner();
   * });
   * ```
   */
  public onBackpressure(
    event: 'backpressure:high' | 'backpressure:low',
    listener: (event: BackpressureThresholdEvent) => void,
  ): () => void;
  public onBackpressure(
    event: 'operation:dropped',
    listener: (event: OperationDroppedEvent) => void,
  ): () => void;
  public onBackpressure(
    event: 'backpressure:paused' | 'backpressure:resumed',
    listener: () => void,
  ): () => void;
  public onBackpressure(
    event:
      | 'backpressure:high'
      | 'backpressure:low'
      | 'backpressure:paused'
      | 'backpressure:resumed'
      | 'operation:dropped',
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- implementation signature must be broad enough to cover all narrowing overloads
    listener: (event?: any) => void,
  ): () => void {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- delegating across the overloaded boundary; runtime dispatch is purely by event string
    return this.syncEngine.onBackpressure(event as any, listener as any);
  }

  // ============================================
  // Full-Text Search API
  // ============================================

  /**
   * Perform a one-shot BM25 search on the server.
   *
   * Searches the specified map using the BM25 ranking algorithm. Every map is
   * full-text indexed automatically on the server — there is no per-map enable
   * flag to set.
   *
   * @param mapName Name of the map to search
   * @param query Search query text
   * @param options Search options
   * @returns Promise resolving to search results sorted by relevance
   *
   * @example
   * ```typescript
   * const results = await client.search<Article>('articles', 'machine learning', {
   *   limit: 20,
   *   minScore: 0.5,
   *   boost: { title: 2.0, body: 1.0 }
   * });
   *
   * for (const result of results) {
   *   console.log(`${result.key}: ${result.value.title} (score: ${result.score})`);
   * }
   * ```
   */
  public async search<T>(
    mapName: string,
    query: string,
    options?: {
      limit?: number;
      minScore?: number;
      boost?: Record<string, number>;
    },
  ): Promise<
    Array<{
      key: string;
      value: T;
      score: number;
      matchedTerms: string[];
    }>
  > {
    return this.syncEngine.search<T>(mapName, query, options);
  }

  // ============================================
  // SQL Query API
  // ============================================

  /**
   * Execute a SQL query on the server via DataFusion.
   *
   * Queries are executed server-side against registered maps.
   * Map names are table names in SQL. Requires the server to have
   * the DataFusion feature enabled and maps registered with schemas.
   *
   * @param query SQL query string (e.g., "SELECT * FROM users WHERE age > 21")
   * @returns Promise resolving to { columns, rows }
   *
   * @example
   * ```typescript
   * const result = await client.sql('SELECT name, age FROM users WHERE age > 21 ORDER BY age');
   * console.log(result.columns); // ['name', 'age']
   * for (const row of result.rows) {
   *   console.log(row[0], row[1]); // name, age
   * }
   * ```
   */
  public async sql(query: string): Promise<SqlQueryResult> {
    return this.syncEngine.sql(query);
  }

  // ============================================
  // Vector Search API
  // ============================================

  /**
   * Perform an ANN vector search on the server using the HNSW index.
   *
   * Sends a VECTOR_SEARCH message and resolves with ranked results.
   * The query vector is transmitted as little-endian f32 bytes and the
   * result vectors (if requested) are decoded back to Float32Array.
   *
   * @param mapName Name of the map / HNSW index to search
   * @param queryVector Query vector as Float32Array or number[]
   * @param options Search options (k, efSearch, minScore, etc.)
   * @returns Promise resolving to ranked VectorSearchClientResult[]
   *
   * @example
   * ```typescript
   * const results = await client.vectorSearch('notes', new Float32Array([0.1, 0.2, 0.3]), { k: 5 });
   * for (const r of results) {
   *   console.log(r.key, r.score);
   * }
   * ```
   */
  public async vectorSearch(
    mapName: string,
    queryVector: Float32Array | number[],
    options?: VectorSearchClientOptions,
  ): Promise<VectorSearchClientResult[]> {
    return this.syncEngine.vectorSearch(mapName, queryVector, options);
  }

  /**
   * Perform a tri-hybrid search (exact + fullText + semantic via RRF).
   *
   * @param mapName Name of the map to search
   * @param queryText Search query text
   * @param options Search options (methods, k, queryVector, predicate, etc.)
   * @returns Promise resolving to ranked HybridSearchClientResult[]
   */
  public async hybridSearch(
    mapName: string,
    queryText: string,
    options?: HybridSearchClientOptions,
  ): Promise<HybridSearchClientResult[]> {
    return this.syncEngine.hybridSearch(mapName, queryText, options);
  }

  // ============================================
  // Live Search API
  // ============================================

  /**
   * Subscribe to live search results with real-time updates.
   *
   * Unlike the one-shot `search()` method, `searchSubscribe()` returns a handle
   * that receives delta updates (ENTER/UPDATE/LEAVE) when documents change.
   * This is ideal for live search UIs that need to reflect data changes.
   *
   * @param mapName Name of the map to search
   * @param query Search query text
   * @param options Search options (limit, minScore, boost)
   * @returns SearchHandle for managing the subscription
   *
   * @example
   * ```typescript
   * const handle = client.searchSubscribe<Article>('articles', 'machine learning', {
   *   limit: 20,
   *   minScore: 0.5
   * });
   *
   * // Subscribe to result changes
   * const unsubscribe = handle.subscribe((results) => {
   *   setSearchResults(results);
   * });
   *
   * // Update query dynamically
   * handle.setQuery('deep learning');
   *
   * // Get current snapshot
   * const snapshot = handle.getResults();
   *
   * // Cleanup when done
   * handle.dispose();
   * ```
   */
  public searchSubscribe<T>(
    mapName: string,
    query: string,
    options?: SearchOptions,
  ): SearchHandle<T> {
    return new SearchHandle<T>(this.syncEngine, mapName, query, options);
  }

  /**
   * Subscribe to live tri-hybrid search results with real-time delta updates.
   *
   * Returns a HybridSearchHandle that sends HYBRID_SEARCH_SUB and receives
   * ENTER/UPDATE/LEAVE deltas via HYBRID_SEARCH_UPDATE. Mirrors searchSubscribe()
   * but for the tri-hybrid RRF path.
   *
   * @param mapName Name of the map to search
   * @param queryText Search query text
   * @param options Subscription options (methods, k, queryVector, predicate, etc.)
   * @returns HybridSearchHandle for managing the subscription
   *
   * @example
   * ```typescript
   * const handle = client.hybridSearchSubscribe<Article>('articles', 'machine learning', {
   *   methods: ['fullText', 'semantic'],
   *   k: 20,
   * });
   *
   * const unsubscribe = handle.subscribe((results) => {
   *   setSearchResults(results);
   * });
   *
   * // Cleanup when done
   * handle.dispose();
   * ```
   */
  public hybridSearchSubscribe<T = unknown>(
    mapName: string,
    queryText: string,
    options?: HybridSearchSubscribeOptions,
  ): HybridSearchHandle<T> {
    return new HybridSearchHandle<T>(this.syncEngine, mapName, queryText, options);
  }

  // ============================================
  // Hybrid Query API
  // ============================================

  /**
   * Create a hybrid query combining FTS with traditional filters.
   *
   * Hybrid queries allow combining full-text search predicates (match, matchPhrase, matchPrefix)
   * with traditional filter predicates (eq, gt, lt, contains, etc.) in a single query.
   * Results include relevance scores for FTS ranking.
   *
   * @param mapName Name of the map to query
   * @param filter Hybrid query filter with predicate, where, sort, limit, cursor
   * @returns HybridQueryHandle for managing the subscription
   *
   * @example
   * ```typescript
   * import { Predicates } from '@topgunbuild/core';
   *
   * // Hybrid query: FTS + filter
   * const handle = client.hybridQuery<Article>('articles', {
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
   *   results.forEach(r => console.log(`${r._key}: score=${r._score}`));
   * });
   * ```
   */
  public hybridQuery<T>(mapName: string, filter: HybridQueryFilter = {}): HybridQueryHandle<T> {
    return new HybridQueryHandle<T>(this.syncEngine, mapName, filter);
  }

  // ============================================
  // Entry Processor API
  // ============================================

  /**
   * Execute an entry processor on a single key atomically.
   *
   * Entry processors solve the read-modify-write race condition by executing
   * user-defined logic atomically on the server where the data lives.
   *
   * @param mapName Name of the map
   * @param key Key to process
   * @param processor Processor definition with name, code, and optional args
   * @returns Promise resolving to the processor result
   *
   * @example
   * ```typescript
   * // Increment a counter atomically
   * const result = await client.executeOnKey('stats', 'pageViews', {
   *   name: 'increment',
   *   code: `
   *     const current = value ?? 0;
   *     return { value: current + 1, result: current + 1 };
   *   `,
   * });
   *
   * // Using built-in processor
   * import { BuiltInProcessors } from '@topgunbuild/core';
   * const result = await client.executeOnKey(
   *   'stats',
   *   'pageViews',
   *   BuiltInProcessors.INCREMENT(1)
   * );
   * ```
   */
  /**
   * @deprecated Entry processors require a server-side WASM sandbox that is on
   * the v2.x roadmap. Calling this method throws immediately. See
   * https://topgun.build/docs/roadmap for status.
   */
  public async executeOnKey<V, R = V>(
    _mapName: string,
    _key: string,
    _processor: EntryProcessorDef<V, R>,
  ): Promise<EntryProcessorResult<R>> {
    throw new Error(
      'Entry processors require server-side WASM sandbox execution, which is on the v2.x roadmap. ' +
        'See https://topgun.build/docs/roadmap. The SDK surface will return when the sandbox lands.',
    );
  }

  /**
   * Execute an entry processor on multiple keys.
   *
   * Each key is processed atomically. The operation returns when all keys
   * have been processed.
   *
   * @param mapName Name of the map
   * @param keys Keys to process
   * @param processor Processor definition
   * @returns Promise resolving to a map of key -> result
   *
   * @example
   * ```typescript
   * // Reset multiple counters
   * const results = await client.executeOnKeys(
   *   'stats',
   *   ['pageViews', 'uniqueVisitors', 'bounceRate'],
   *   {
   *     name: 'reset',
   *     code: `return { value: 0, result: value };`, // Returns old value
   *   }
   * );
   *
   * for (const [key, result] of results) {
   *   console.log(`${key}: was ${result.result}, now 0`);
   * }
   * ```
   */
  /**
   * @deprecated Entry processors require a server-side WASM sandbox that is on
   * the v2.x roadmap. Calling this method throws immediately. See
   * https://topgun.build/docs/roadmap for status.
   */
  public async executeOnKeys<V, R = V>(
    _mapName: string,
    _keys: string[],
    _processor: EntryProcessorDef<V, R>,
  ): Promise<Map<string, EntryProcessorResult<R>>> {
    throw new Error(
      'Entry processors require server-side WASM sandbox execution, which is on the v2.x roadmap. ' +
        'See https://topgun.build/docs/roadmap. The SDK surface will return when the sandbox lands.',
    );
  }

  // ============================================
  // Event Journal API
  // ============================================

  /** Cached EventJournalReader instance */
  private journalReader?: EventJournalReader;

  /**
   * Get the Event Journal reader for subscribing to and reading
   * map change events.
   *
   * The Event Journal provides:
   * - Append-only log of all map changes (PUT, UPDATE, DELETE)
   * - Subscription to real-time events
   * - Historical event replay
   * - Audit trail for compliance
   *
   * @returns EventJournalReader instance
   *
   * @example
   * ```typescript
   * const journal = client.getEventJournal();
   *
   * // Subscribe to all events
   * const unsubscribe = journal.subscribe((event) => {
   *   console.log(`${event.type} on ${event.mapName}:${event.key}`);
   * });
   *
   * // Subscribe to specific map
   * journal.subscribe(
   *   (event) => console.log('User changed:', event.key),
   *   { mapName: 'users' }
   * );
   *
   * // Read historical events
   * const events = await journal.readFrom(0n, 100);
   * ```
   */
  public getEventJournal(): EventJournalReader {
    if (!this.journalReader) {
      this.journalReader = new EventJournalReader(this.syncEngine);
    }
    return this.journalReader;
  }

  // ============================================
  // Conflict Resolver API
  // ============================================

  /**
   * Get the conflict resolver client for registering custom merge resolvers.
   *
   * Conflict resolvers allow you to customize how merge conflicts are handled
   * on the server. You can implement business logic like:
   * - First-write-wins for booking systems
   * - Numeric constraints (non-negative, min/max)
   * - Owner-only modifications
   * - Custom merge strategies
   *
   * @returns ConflictResolverClient instance
   *
   * @example
   * ```typescript
   * const resolvers = client.getConflictResolvers();
   *
   * // Register a first-write-wins resolver
   * await resolvers.register('bookings', {
   *   name: 'first-write-wins',
   *   code: `
   *     if (context.localValue !== undefined) {
   *       return { action: 'reject', reason: 'Slot already booked' };
   *     }
   *     return { action: 'accept', value: context.remoteValue };
   *   `,
   *   priority: 100,
   * });
   *
   * // Subscribe to merge rejections
   * resolvers.onRejection((rejection) => {
   *   console.log(`Merge rejected: ${rejection.reason}`);
   *   // Optionally refresh local state
   * });
   *
   * // List registered resolvers
   * const registered = await resolvers.list('bookings');
   * console.log('Active resolvers:', registered);
   *
   * // Unregister when done
   * await resolvers.unregister('bookings', 'first-write-wins');
   * ```
   */
  public getConflictResolvers() {
    return this.syncEngine.getConflictResolverClient();
  }

  /**
   * Get the per-record sync-state tracker — projects opLog mutations,
   * connection state, and merge rejections into a four-state tag per
   * (mapName, key). Used by React hooks (`useSyncState`, the
   * `*WithSyncState` companions) and by advanced consumers reading
   * sync state outside a query context.
   */
  public getRecordSyncStateTracker() {
    return this.syncEngine.getRecordSyncStateTracker();
  }
}
