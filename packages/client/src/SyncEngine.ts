import { HLC, LWWMap, ORMap, deserialize } from '@topgunbuild/core';
import type { EntryProcessorDef, EntryProcessorResult, SearchOptions } from '@topgunbuild/core';
import type { LWWRecord, ORMapRecord, Timestamp } from '@topgunbuild/core';
import type {
  AuthFailMessage,
  AuthMessage,
  AuthAckMessage,
  DeviceHelloMessage,
  DeviceAckMessage,
  OpAckMessage,
  OpRejectedMessage,
  ErrorMessage,
  QueryRespMessage,
  QueryUpdateMessage,
  ServerEventMessage,
  ServerBatchEventMessage,
  GcPruneMessage,
  BatchMessage,
} from '@topgunbuild/core';
import type { IStorageAdapter, StorageMutation } from './IStorageAdapter';
import { QueryHandle } from './QueryHandle';
import type { QueryFilter } from './QueryHandle';
import type { HybridQueryHandle, HybridQueryFilter } from './HybridQueryHandle';
import { TopicHandle } from './TopicHandle';
import { logger } from './utils/logger';
import { SyncStateMachine, StateChangeEvent } from './SyncStateMachine';
import { SyncState } from './SyncState';
import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from './BackpressureConfig';
import { DEFAULT_BACKPRESSURE_CONFIG } from './BackpressureConfig';
import type { IConnectionProvider } from './types';
import { ConflictResolverClient } from './ConflictResolverClient';
import { AuthRequiredError } from './errors/AuthRequiredError';
import { RecordSyncStateTracker } from './RecordSyncState';
import {
  WebSocketManager,
  BackpressureController,
  QueryManager,
  TopicManager,
  LockManager,
  WriteConcernManager,
  CounterManager,
  EntryProcessorClient,
  SearchClient,
  SqlClient,
  VectorSearchClient,
  HybridSearchClient,
  MerkleSyncHandler,
  ORMapSyncHandler,
  MessageRouter,
  registerClientMessageHandlers,
} from './sync';
import type {
  SearchResult,
  SqlQueryResult,
  VectorSearchClientOptions,
  VectorSearchClientResult,
  HybridSearchClientOptions,
  HybridSearchClientResult,
  IMessageRouter,
} from './sync';

// Re-export SearchResult and SqlQueryResult from sync module for backwards compatibility
export type { SearchResult, SqlQueryResult } from './sync';

export interface OpLogEntry {
  id: string; // Unique ID for the operation
  mapName: string;
  opType: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE';
  key: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op log entries hold records with unknown value type; type is determined by the map the entry belongs to
  record?: LWWRecord<any>; // LWW Put/Remove (Remove has null value)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMapRecord value type erased in the op log; maps use their own generic at runtime
  orRecord?: ORMapRecord<any>; // ORMap Add
  orTag?: string; // ORMap Remove (Tombstone tag)
  timestamp: Timestamp; // HLC timestamp of the operation
  synced: boolean; // True if this operation has been successfully pushed to the server
}

export interface HeartbeatConfig {
  intervalMs: number; // Default: 5000 (5 seconds)
  timeoutMs: number; // Default: 15000 (15 seconds)
  enabled: boolean; // Default: true
}

export interface BackoffConfig {
  /** Initial delay in milliseconds (default: 1000) */
  initialDelayMs: number;
  /** Maximum delay in milliseconds (default: 30000) */
  maxDelayMs: number;
  /** Multiplier for exponential backoff (default: 2) */
  multiplier: number;
  /** Whether to add random jitter to delay (default: true) */
  jitter: boolean;
  /**
   * Maximum number of reconnect attempts before giving up (default: Infinity).
   * With the default, transient/network outages are retried indefinitely at the
   * capped backoff interval (the offline-first contract). Set a finite number for
   * a bounded policy: on exhaustion the engine transitions to SyncState.ERROR and
   * fires onConnectionStateChange with the terminal state — it does NOT retry
   * forever and does NOT fail silently.
   */
  maxRetries: number;
}

export interface TopicQueueConfig {
  /** Maximum queued topic messages when offline (default: 100) */
  maxSize: number;
  /** Strategy when queue is full: 'drop-oldest' | 'drop-newest' (default: 'drop-oldest') */
  strategy: 'drop-oldest' | 'drop-newest';
}

const DEFAULT_TOPIC_QUEUE_CONFIG: TopicQueueConfig = {
  maxSize: 100,
  strategy: 'drop-oldest',
};

export interface SyncEngineConfig {
  nodeId: string;
  /** Connection provider for WebSocket connections */
  connectionProvider: IConnectionProvider;
  storageAdapter: IStorageAdapter;
  reconnectInterval?: number;
  heartbeat?: Partial<HeartbeatConfig>;
  backoff?: Partial<BackoffConfig>;
  backpressure?: Partial<BackpressureConfig>;
  /** Configuration for offline topic message queue */
  topicQueue?: Partial<TopicQueueConfig>;
  /**
   * Invoked when the server sends AUTH_REQUIRED but no token / token provider
   * is configured. Without this hook the client parks in AUTHENTICATING forever
   * with only an info-level log line; the callback lets integrators detect and
   * react (prompt for login, call setAuthToken, etc.).
   */
  onAuthRequired?: (error: AuthRequiredError) => void;
}

// NOTE: reconnect/backoff defaults live in ONE place — SingleServerProvider's
// DEFAULT_CONFIG (the component that actually executes reconnect). SyncEngine no
// longer keeps a parallel BackoffConfig; `config.backoff` is mapped straight to the
// connection provider in TopGunClient. This avoids the "two sources of truth, one
// dead" trap where editing the wrong half changed nothing.

/**
 * Extracts the server epoch a pushed event confirms, for the confirmed-apply cursor.
 *
 * Forward-compatible with the prune-wiring epoch stamping: the server does not yet
 * stamp an `epoch` on live delta events, so this returns 0 today (a zero epoch is
 * dropped by `applyMapCoverage`, so the push path is inert until the wire stamps
 * one). `handleServerEvent`/`handleServerBatchEvent` route this value through
 * `applyMapCoverage(mapName, epoch)` — the cross-map min-barrier — NEVER through
 * `emitConfirmedApply` directly, so when epoch stamping on push events lands, a
 * single map's live event cannot license a device-wide ACK past a tombstone some
 * OTHER held map has not received.
 */
function epochOfServerEvent(payload: unknown): number {
  const epoch = (payload as { epoch?: unknown } | null)?.epoch;
  return typeof epoch === 'number' && Number.isFinite(epoch) && epoch > 0 ? epoch : 0;
}

// Eager per-OR-Map existence marker. Written at the FIRST durable OR-state write
// for a map (both the local-write commit AND the server-origin persist helpers),
// so the covering-epoch held-set snapshot can discover EVERY persisted OR-Map —
// including an add-only map that never wrote a `:tombstones` meta-key. Without
// this, a lazily-opened add-only store is invisible to the snapshot and the
// device ACK can advance past its un-received tombstones (the cross-map
// resurrection vector). See `computeHeldOrMapNames` / `ensureOrMapMarker`.
const orMapMarkerKey = (mapName: string): string => `__sys__:${mapName}:ormap`;
// Set ONCE after the legacy-store backfill scan succeeds. Its ABSENCE forces the
// backfill to (re-)run inside `computeHeldOrMapNames`; a scan failure leaves it
// unset so the next connection retries — and the throw fail-closes this
// connection's ACKs. Deliberately NOT of the form `__sys__:*:ormap|tombstones`
// so it is never matched as a held-map name by the enumeration regex.
const ORMAP_BACKFILL_DONE_KEY = '__sys__:ormapBackfillDone';
// Discovers a held OR-Map name from either meta-marker: the eager `:ormap`
// existence marker (post-fix + backfilled stores) OR the legacy `:tombstones`
// key (belt-and-suspenders — a pre-fix store that HAS a tombstone but somehow
// escaped backfill is still discoverable).
const HELD_ORMAP_META_RE = /^__sys__:(.+):(?:ormap|tombstones)$/;

export class SyncEngine {
  private readonly nodeId: string;
  private readonly storageAdapter: IStorageAdapter;
  private readonly hlc: HLC;
  private readonly stateMachine: SyncStateMachine;
  private readonly heartbeatConfig: HeartbeatConfig;

  // WebSocketManager handles all connection/websocket operations
  private readonly webSocketManager: WebSocketManager;

  // QueryManager handles all query operations
  private readonly queryManager: QueryManager;

  // TopicManager handles all topic (pub/sub) operations
  private readonly topicManager: TopicManager;

  // LockManager handles distributed lock operations
  private readonly lockManager: LockManager;

  // WriteConcernManager handles write concern tracking
  private readonly writeConcernManager: WriteConcernManager;

  // CounterManager handles PN counter operations
  private readonly counterManager: CounterManager;

  // EntryProcessorClient handles entry processor operations
  private readonly entryProcessorClient: EntryProcessorClient;

  // SearchClient handles full-text search operations
  private readonly searchClient: SearchClient;

  // SqlClient handles server-side SQL query execution via DataFusion
  private readonly sqlClient: SqlClient;

  // VectorSearchClient handles ANN vector search requests
  private readonly vectorSearchClient: VectorSearchClient;

  // HybridSearchClient handles tri-hybrid RRF search requests
  private readonly hybridSearchClient: HybridSearchClient;

  // MerkleSyncHandler handles LWWMap sync protocol messages
  private readonly merkleSyncHandler: MerkleSyncHandler;

  // ORMapSyncHandler handles ORMap sync protocol messages
  private readonly orMapSyncHandler: ORMapSyncHandler;

  // MessageRouter handles type-based message routing
  private readonly messageRouter: IMessageRouter;

  private opLog: OpLogEntry[] = [];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- maps registry holds heterogeneous LWWMap and ORMap instances; value types differ per map name
  private maps: Map<string, LWWMap<any, any> | ORMap<any, any>> = new Map();
  private lastSyncTimestamp: number = 0;
  // Highest server epoch this client has confirmed to the server via CLIENT_APPLY_ACK.
  // The confirmed-apply cursor is cumulative-monotonic: a non-advancing epoch is never
  // re-sent. Distinct from lastSyncTimestamp (a delta-sync optimization hint) — this is
  // the apply-not-receive confirmation the server's per-device causal frontier reads.
  private lastAckedEpoch: number = 0;
  // Per-OR-Map coverage: highest covering epoch durably applied for that map on
  // THIS connection. The device-wide CLIENT_APPLY_ACK is gated by the MIN across
  // `heldOrMapNames` (never a single map's own coverage) — the cross-map
  // covering-epoch ACK-inflation fix. Reset every connection in startMerkleSync.
  private orMapCoverage: Map<string, number> = new Map();
  // Consistent snapshot of every OR-Map this device holds locally (open
  // instances UNION persisted-but-not-yet-opened stores), taken ONCE per
  // connection in startMerkleSync BEFORE any covering-epoch ACK can fire. `null`
  // before the first sync of a connection — applyMapCoverage treats that (and an
  // empty snapshot) as "nothing to confirm coverage over" and never ACKs.
  private heldOrMapNames: Set<string> | null = null;
  // Fail-closed marker: true when this connection's held-set enumeration FAILED
  // (storage adapter error). While set, applyMapCoverage never emits an ACK — a
  // partial held-set (in-memory maps only, persisted stores unknown) would run
  // the min-barrier over an incomplete universe, which is the exact cross-map
  // ACK-inflation hole reopened through a different door. Reset per connection.
  private heldSetIncomplete = false;
  // De-noises the fail-closed path: warn once per connection, not per apply.
  private heldSetIncompleteWarned = false;
  // Once-per-SESSION guard for the eager `:ormap` existence marker: names whose
  // marker we have already ensured this process lifetime. The marker is durable,
  // so re-ensuring after a restart is a harmless idempotent setMeta; this Set
  // just keeps the common OR-write path from issuing a redundant marker write on
  // every add/remove. NOT reset per connection (the durable marker outlives a
  // reconnect).
  private markedOrMaps: Set<string> = new Set();
  private authToken: string | null = null;
  private tokenProvider: (() => Promise<string | null>) | null = null;
  private onAuthRequired: ((error: AuthRequiredError) => void) | null = null;

  // Server-issued device identity. The deviceToken is an opaque credential the
  // client presents on AUTH so the server can rebind the same device identity
  // across reconnects/restarts; the server returns (and may rotate) it on
  // AUTH_ACK. Both are persisted durably via the storage adapter's meta store so
  // they survive process restarts. The token is opaque — never parsed by the client.
  private deviceToken: string | null = null;
  private deviceId: string | null = null;
  // Memoized in-flight load of the persisted device credential, so the boot load
  // (loadOpLog) and the auth path share a single read and the credential is
  // guaranteed available before the first AUTH is presented.
  private deviceCredentialLoadPromise: Promise<void> | null = null;
  // True while a token-less DEVICE_HELLO awaits its DEVICE_ACK. Used to infer a legacy
  // (pre-device-identity) server: any non-DEVICE_ACK message (or the grace timeout)
  // while this is set means proceed auth-optional without a deviceId.
  private deviceAckPending = false;

  // Grace timer: gives the server a bounded window to send AUTH_REQUIRED after WS open.
  // If AUTH_REQUIRED arrives, the timer is cancelled and existing auth behaviour runs.
  // If the window expires without AUTH_REQUIRED, the auth-optional fast-path fires.
  private authRequiredGraceTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly AUTH_REQUIRED_GRACE_MS = 500;

  // BackpressureController handles all backpressure operations
  private readonly backpressureConfig: BackpressureConfig;
  private readonly backpressureController: BackpressureController;

  // Conflict Resolver client
  private readonly conflictResolverClient: ConflictResolverClient;

  // Per-record sync-state tracker — projects opLog mutations + connection
  // state + MergeRejection stream into a four-state tag per (mapName, key).
  private readonly recordSyncStateTracker: RecordSyncStateTracker;

  constructor(config: SyncEngineConfig) {
    // Validate config: connectionProvider is required
    if (!config.connectionProvider) {
      throw new Error('SyncEngine requires connectionProvider');
    }

    this.nodeId = config.nodeId;
    this.storageAdapter = config.storageAdapter;
    this.onAuthRequired = config.onAuthRequired ?? null;
    this.hlc = new HLC(this.nodeId);

    // Initialize state machine
    this.stateMachine = new SyncStateMachine();

    // Initialize heartbeat config with defaults
    this.heartbeatConfig = {
      intervalMs: config.heartbeat?.intervalMs ?? 5000,
      timeoutMs: config.heartbeat?.timeoutMs ?? 15000,
      enabled: config.heartbeat?.enabled ?? true,
    };

    // Merge backpressure config with defaults
    this.backpressureConfig = {
      ...DEFAULT_BACKPRESSURE_CONFIG,
      ...config.backpressure,
    };

    // Initialize BackpressureController with shared opLog reference
    this.backpressureController = new BackpressureController({
      config: this.backpressureConfig,
      opLog: this.opLog, // Pass reference, not copy
      // Durable drop: when drop-oldest evicts an unsynced op from the in-memory opLog, also
      // delete it from storage so it cannot resurrect on the next reload (memory/disk agree).
      // op.id is a stringified integer; coerce to the numeric storage id at this boundary.
      onOpDropped: (opId: string) => {
        const numericId = parseInt(opId, 10);
        if (!isNaN(numericId)) {
          this.storageAdapter
            .deleteOp(numericId)
            .catch((err) =>
              logger.error({ err, opId }, 'Failed to delete dropped op from storage'),
            );
        }
      },
    });

    // Merge topic queue config with defaults to ensure consistent backpressure behavior
    const topicQueueConfig: TopicQueueConfig = {
      ...DEFAULT_TOPIC_QUEUE_CONFIG,
      ...config.topicQueue,
    };

    // Initialize WebSocketManager with callbacks to SyncEngine
    this.webSocketManager = new WebSocketManager({
      connectionProvider: config.connectionProvider,
      stateMachine: this.stateMachine,
      heartbeatConfig: this.heartbeatConfig,
      onMessage: (msg) => this.handleServerMessage(msg),
      onConnected: () => this.handleConnectionEstablished(),
      onDisconnected: () => this.handleConnectionLost(),
      // No onReconnected: re-arm + auth for both initial and reconnect is driven
      // once by handleConnectionEstablished (provider 'connected'), which fires on
      // every opened socket. A separate reconnect auth path double-sent AUTH (F7).
    });

    // Initialize QueryManager with callbacks
    this.queryManager = new QueryManager({
      storageAdapter: this.storageAdapter,
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize TopicManager with callbacks
    this.topicManager = new TopicManager({
      topicQueueConfig,
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize LockManager with callbacks
    this.lockManager = new LockManager({
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      isAuthenticated: () => this.isAuthenticated(),
      isOnline: () => this.isOnline(),
    });

    // Initialize WriteConcernManager for distributed PN counter operations
    this.writeConcernManager = new WriteConcernManager({
      defaultTimeout: 5000,
    });

    // Initialize CounterManager for distributed PN counter operations
    this.counterManager = new CounterManager({
      sendMessage: (msg) => this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize EntryProcessorClient for server-side entry processing
    this.entryProcessorClient = new EntryProcessorClient({
      sendMessage: (msg, key) =>
        key !== undefined ? this.sendMessage(msg, key) : this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize SearchClient for full-text search operations
    this.searchClient = new SearchClient({
      sendMessage: (msg) => this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize SqlClient for server-side SQL query execution
    this.sqlClient = new SqlClient({
      sendMessage: (msg) => this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize VectorSearchClient for ANN vector search requests
    this.vectorSearchClient = new VectorSearchClient({
      sendMessage: (msg) => this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize HybridSearchClient for tri-hybrid RRF search requests
    this.hybridSearchClient = new HybridSearchClient({
      sendMessage: (msg) => this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize MerkleSyncHandler for LWWMap sync protocol
    this.merkleSyncHandler = new MerkleSyncHandler({
      getMap: (name) => this.maps.get(name),
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      storageAdapter: this.storageAdapter,
      hlc: this.hlc,
      onTimestampUpdate: async (ts) => {
        this.hlc.update(ts);
        this.lastSyncTimestamp = ts.millis;
        await this.saveOpLog();
      },
      resetMap: (name) => this.resetMap(name),
    });

    // Initialize ORMapSyncHandler for ORMap sync protocol
    this.orMapSyncHandler = new ORMapSyncHandler({
      getMap: (name) => this.maps.get(name),
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      hlc: this.hlc,
      onTimestampUpdate: async (ts) => {
        this.hlc.update(ts);
        this.lastSyncTimestamp = ts.millis;
        await this.saveOpLog();
      },
      // Persist server-origin ORMap merges (merkle leaf + diff) so they survive offline
      // reload — same canonical helpers as the local-write + applyServerEvent paths.
      persistKey: (name, key) => this.persistORMapKey(name, key),
      persistTombstones: (name) => this.persistORMapTombstones(name),
      // Fold this map's covering epoch into the device-wide MIN across every
      // held OR-Map (see applyMapCoverage) AFTER its data is durably applied
      // (including on an empty diff), so the server's per-device cursor advances
      // only once every held map has proven delivery — never off a single map's
      // sync completion.
      onCoveringEpochApplied: (mapName, epoch) => this.applyMapCoverage(mapName, epoch),
    });

    // Initialize Conflict Resolver client
    this.conflictResolverClient = new ConflictResolverClient(this);

    // Initialize per-record sync-state tracker. Wire connection-state and
    // merge-rejection subscriptions through the tracker's disposer registry
    // so they are torn down with dispose() in close().
    this.recordSyncStateTracker = new RecordSyncStateTracker(this.stateMachine.getState());
    const offState = this.stateMachine.onStateChange((event) => {
      this.recordSyncStateTracker.onConnectionStateChange(event.to);
    });
    this.recordSyncStateTracker.registerDisposer(offState);
    const offRejection = this.conflictResolverClient.onRejection((rejection) => {
      this.recordSyncStateTracker.onRejection(rejection);
    });
    this.recordSyncStateTracker.registerDisposer(offRejection);

    // Initialize MessageRouter and register all handlers
    this.messageRouter = new MessageRouter({
      onUnhandled: (msg) => logger.warn({ type: msg?.type }, 'Unhandled message type'),
    });
    registerClientMessageHandlers(
      this.messageRouter,
      {
        sendAuth: () => this.sendAuth(),
        handleAuthRequired: () => this.handleAuthRequired(),
        handleAuthAck: (msg) => this.handleAuthAck(msg),
        handleDeviceAck: (msg) => this.handleDeviceAck(msg),
        handleAuthFail: (msg) => this.handleAuthFail(msg),
        handleOpAck: (msg) => this.handleOpAck(msg),
        handleQueryResp: (msg) => this.handleQueryResp(msg),
        handleQueryUpdate: (msg) => this.handleQueryUpdate(msg),
        handleServerEvent: (msg) => this.handleServerEvent(msg),
        handleServerBatchEvent: (msg) => this.handleServerBatchEvent(msg),
        handleGcPrune: (msg) => this.handleGcPrune(msg),
        handleOpRejected: (msg) => this.handleOpRejected(msg),
        handleError: (msg) => this.handleError(msg),
      },
      {
        topicManager: this.topicManager,
        lockManager: this.lockManager,
        counterManager: this.counterManager,
        entryProcessorClient: this.entryProcessorClient,
        conflictResolverClient: this.conflictResolverClient,
        searchClient: this.searchClient,
        sqlClient: this.sqlClient,
        vectorSearchClient: this.vectorSearchClient,
        hybridSearchClient: this.hybridSearchClient,
        merkleSyncHandler: this.merkleSyncHandler,
        orMapSyncHandler: this.orMapSyncHandler,
      },
    );

    // Start connection
    this.webSocketManager.connect();

    this.loadOpLog();

    // Kick off the device-credential read independently of loadOpLog so it does
    // not delay the op-log rebuild (loadOpLog resets this.opLog). sendAuth awaits
    // the same memoized promise before presenting the credential.
    void this.ensureDeviceCredentialLoaded();
  }

  // ============================================
  // Connection Callbacks (from WebSocketManager)
  // ============================================

  /**
   * Called when connection is established (initial or reconnect).
   */
  private handleConnectionEstablished(): void {
    if (this.authToken || this.tokenProvider) {
      // Client already has credentials — send auth immediately without waiting
      // for AUTH_REQUIRED (preserves today's behaviour; matches servers that
      // expect the client to speak first, e.g. token auto-auth flows).
      logger.info('Connection established. Sending auth...');
      this.stateMachine.transition(SyncState.AUTHENTICATING);
      this.sendAuth();
      return;
    }

    // Token-less connect (no token, no provider). Present the device credential on a
    // dedicated DEVICE_HELLO frame — NOT an empty-token AUTH, which a real JWT server
    // treats as a JWT attempt, fails, and tears the connection down. DEVICE_HELLO is a
    // distinct non-AUTH frame: a device-aware NO_AUTH server present-or-mints and replies
    // DEVICE_ACK; a JWT server silently drops it and (separately) sends AUTH_REQUIRED, so
    // the connection survives and Case 3 (supply a token later) still works; a legacy
    // server ignores it. Arm the grace timer so a legacy/no-reply server still proceeds
    // auth-optional instead of hanging.
    logger.info(
      { graceMs: this.AUTH_REQUIRED_GRACE_MS },
      'Connection established (token-less). Presenting device credential; waiting briefly for DEVICE_ACK...',
    );
    this.stateMachine.transition(SyncState.AUTHENTICATING);
    this.deviceAckPending = true;
    this.sendDeviceHello();
    this.authRequiredGraceTimer = setTimeout(() => {
      this.authRequiredGraceTimer = null;
      this.deviceAckPending = false;
      this.completeAuthOptionalConnection();
    }, this.AUTH_REQUIRED_GRACE_MS);
  }

  /**
   * Called when connection is lost.
   */
  private handleConnectionLost(): void {
    // Cancel any pending grace timer to prevent it firing during reconnection
    // and driving state transitions on a stale connection.
    if (this.authRequiredGraceTimer) {
      clearTimeout(this.authRequiredGraceTimer);
      this.authRequiredGraceTimer = null;
    }
    // WebSocketManager already stopped heartbeat and transitioned state
    // SyncEngine can do additional cleanup if needed
  }

  /**
   * Auth-optional fast path: server did not demand authentication within the
   * grace window, so drive the state machine through AUTHENTICATING → SYNCING → CONNECTED
   * without sending an AUTH frame. Runs the same post-auth wiring as handleAuthAck()
   * (heartbeat start, merkle sync kickoff, query/topic resubscribe, backoff reset).
   */
  private completeAuthOptionalConnection(): void {
    // Clear grace timer (idempotent — safe if called from timer callback itself).
    if (this.authRequiredGraceTimer) {
      clearTimeout(this.authRequiredGraceTimer);
      this.authRequiredGraceTimer = null;
    }

    // Only run if still in a pre-auth state — guard against races where
    // AUTH_REQUIRED or disconnect arrived concurrently.
    const state = this.stateMachine.getState();
    if (state !== SyncState.CONNECTING && state !== SyncState.AUTHENTICATING) {
      return;
    }

    logger.info(
      'No DEVICE_ACK received within grace window — assuming auth-optional legacy server.',
    );
    this.deviceAckPending = false;
    // Traverse the canonical pre-auth → ready path (no new state transitions added).
    this.stateMachine.transition(SyncState.AUTHENTICATING);
    // No message: proceed without a deviceId (degraded-to-legacy).
    this.handleAuthAck(); // Reuses existing SYNCING → CONNECTED wiring.
  }

  /**
   * AUTH_REQUIRED received from server: cancel any grace timer and send auth
   * if a token is available, otherwise park in AUTHENTICATING waiting for
   * setAuthToken(). Preserves existing no-token-but-server-requires-auth behaviour.
   *
   * Guards the state transition: only moves to AUTHENTICATING if the current
   * state is CONNECTING. This prevents AUTHENTICATING → AUTHENTICATING when
   * AUTH_REQUIRED arrives after the token-configured path has already transitioned
   * (e.g., a server protocol ping or session re-auth frame), which would otherwise
   * produce an "Invalid state transition" log and violate AC #4.
   */
  private handleAuthRequired(): void {
    // The server demands JWT auth — abandon the opportunistic device-ack wait.
    this.deviceAckPending = false;
    if (this.authRequiredGraceTimer) {
      clearTimeout(this.authRequiredGraceTimer);
      this.authRequiredGraceTimer = null;
    }
    // Only transition to AUTHENTICATING from CONNECTING. If we are already in
    // AUTHENTICATING (token-configured path), skip the transition to avoid an
    // invalid self-transition.
    if (this.stateMachine.getState() === SyncState.CONNECTING) {
      this.stateMachine.transition(SyncState.AUTHENTICATING);
    }
    if (this.authToken || this.tokenProvider) {
      this.sendAuth();
    } else {
      // Without a token the SyncEngine parks in AUTHENTICATING. Surface the
      // condition loudly: warn-level log + typed callback so integrators can
      // react (prompt for login, call setAuthToken, redirect, etc.) instead
      // of debugging a silent connection.
      const error = new AuthRequiredError();
      logger.warn(
        { code: error.code },
        'AUTH_REQUIRED received but no token configured. Call client.setAuthToken(token) or configure config.auth/config.onAuthRequired.',
      );
      if (this.onAuthRequired) {
        try {
          this.onAuthRequired(error);
        } catch (callbackErr) {
          logger.error({ err: callbackErr }, 'onAuthRequired callback threw');
        }
      }
    }
  }

  // ============================================
  // State Machine Public API
  // ============================================

  /**
   * Get the current connection state
   */
  getConnectionState(): SyncState {
    return this.stateMachine.getState();
  }

  /**
   * Subscribe to connection state changes
   * @returns Unsubscribe function
   */
  onConnectionStateChange(listener: (event: StateChangeEvent) => void): () => void {
    return this.stateMachine.onStateChange(listener);
  }

  /**
   * Get state machine history for debugging
   */
  getStateHistory(limit?: number): StateChangeEvent[] {
    return this.stateMachine.getHistory(limit);
  }

  // ============================================
  // Internal State Helpers (replace boolean flags)
  // ============================================

  /**
   * Check if WebSocket is connected (but may not be authenticated yet)
   */
  private isOnline(): boolean {
    const state = this.stateMachine.getState();
    return (
      state === SyncState.CONNECTING ||
      state === SyncState.AUTHENTICATING ||
      state === SyncState.SYNCING ||
      state === SyncState.CONNECTED
    );
  }

  /**
   * Check if fully authenticated and ready for operations
   */
  private isAuthenticated(): boolean {
    const state = this.stateMachine.getState();
    return state === SyncState.SYNCING || state === SyncState.CONNECTED;
  }

  /**
   * Check if fully connected and synced
   */
  private isConnected(): boolean {
    return this.stateMachine.getState() === SyncState.CONNECTED;
  }

  // ============================================
  // Message Sending (delegates to WebSocketManager)
  // ============================================

  /**
   * Send a message through the current connection.
   * Delegates to WebSocketManager.
   */
  private sendMessage(message: unknown, key?: string): boolean {
    return this.webSocketManager.sendMessage(message, key);
  }

  // ============================================
  // Op Log Management
  // ============================================

  /**
   * Load the persisted server-issued device credential (token + id) from durable
   * meta storage into memory, at most once. Runs on boot (via loadOpLog) and is
   * also awaited lazily in sendAuth to close the race where the connection opens
   * before loadOpLog resolves — so a persisted deviceToken is always presented.
   */
  private ensureDeviceCredentialLoaded(): Promise<void> {
    if (!this.deviceCredentialLoadPromise) {
      this.deviceCredentialLoadPromise = this.loadDeviceCredential();
    }
    return this.deviceCredentialLoadPromise;
  }

  private async loadDeviceCredential(): Promise<void> {
    try {
      const token = await this.storageAdapter.getMeta('deviceToken');
      if (typeof token === 'string' && token.length > 0) {
        this.deviceToken = token;
      }
      const id = await this.storageAdapter.getMeta('deviceId');
      if (typeof id === 'string' && id.length > 0) {
        this.deviceId = id;
      }
    } catch (err) {
      // A storage read failure must not block auth — the server will present-or-mint
      // a fresh identity if we present nothing (fail-open).
      logger.warn({ err }, 'Failed to load persisted device credential');
    }
  }

  /**
   * Persist a device credential carried on AUTH_ACK (credentialed path) or DEVICE_ACK
   * (token-less path). `deviceToken` is present only when the server minted or rotated
   * the credential; when absent (a plain re-bind of an already-valid token) the existing
   * token is kept, never cleared.
   */
  private persistDeviceCredential(message: AuthAckMessage | DeviceAckMessage): void {
    // The credential is now authoritative in memory; mark the boot-time load
    // satisfied so it cannot later overwrite these values.
    if (!this.deviceCredentialLoadPromise) {
      this.deviceCredentialLoadPromise = Promise.resolve();
    }
    if (message.deviceId && message.deviceId !== this.deviceId) {
      this.deviceId = message.deviceId;
      this.storageAdapter
        .setMeta('deviceId', message.deviceId)
        .catch((err) => logger.warn({ err }, 'Failed to persist deviceId'));
    }
    if (message.deviceToken) {
      this.deviceToken = message.deviceToken;
      this.storageAdapter
        .setMeta('deviceToken', message.deviceToken)
        .catch((err) => logger.warn({ err }, 'Failed to persist deviceToken'));
    }
  }

  private async loadOpLog(): Promise<void> {
    const storedTimestamp = await this.storageAdapter.getMeta('lastSyncTimestamp');
    if (storedTimestamp) {
      this.lastSyncTimestamp = storedTimestamp;
    }

    const pendingOps = await this.storageAdapter.getPendingOps();
    // Clear and push to existing array (preserves BackpressureController reference)
    this.opLog.length = 0;
    for (const op of pendingOps) {
      const restored = {
        ...op,
        id: String(op.id),
        synced: false,
      } as unknown as OpLogEntry;
      this.opLog.push(restored);
      // Surface restored pending ops to the per-record sync-state tracker so
      // they project to 'pending' or 'local-only' immediately on engine boot.
      this.recordSyncStateTracker.onAppend(restored);
    }

    if (this.opLog.length > 0) {
      logger.info({ count: this.opLog.length }, 'Loaded pending operations from local storage');
    }
  }

  private async saveOpLog(): Promise<void> {
    await this.storageAdapter.setMeta('lastSyncTimestamp', this.lastSyncTimestamp);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- map value type is erased in the registry; callers get typed instances via TopGunClient.getMap/getORMap overloads
  public registerMap(mapName: string, map: LWWMap<any, any> | ORMap<any, any>): void {
    this.maps.set(mapName, map);
    if (map instanceof ORMap && this.heldOrMapNames && !this.heldOrMapNames.has(mapName)) {
      // A map opened AFTER this connection's held-set snapshot joins the barrier
      // with coverage 0 — it blocks further ACK advance until it syncs, but any
      // PREVIOUSLY-emitted higher ACK deliberately stands unretracted. That is
      // sound because the only map this path can add is a GENUINELY-NEW one: it
      // is empty at open (no prior local state, no tombstones it could be
      // missing), so there is nothing an already-emitted ACK could have
      // over-claimed about it and nothing to resurrect. The dangerous sibling
      // case — a PERSISTED store from a prior session that the snapshot failed
      // to see — is NOT handled here and never reaches this path: enumeration
      // failure fail-closes the whole connection's ACKs via heldSetIncomplete
      // (see startMerkleSync), and a successful enumeration now puts EVERY
      // persisted OR-Map in the snapshot before the first ACK can fire — every
      // durably-written OR-Map carries an eager `:ormap` existence marker (both
      // write seams) and legacy pre-marker stores are stamped by the one-time
      // backfill, so even an add-only persisted store (no `:tombstones` key) is
      // discovered. A map surfacing here is therefore genuinely-new, not a missed
      // persisted one.
      this.heldOrMapNames.add(mapName);
      if (!this.orMapCoverage.has(mapName)) {
        this.orMapCoverage.set(mapName, 0);
      }
    }
  }

  /**
   * The consistent snapshot of every OR-Map this device holds locally: OR-Map
   * instances already registered in `this.maps`, UNION the OR-Map stores the
   * storage adapter has persisted from a prior session but this process has not
   * (yet) instantiated via `getORMap()`. Enumerated from the reserved OR-Map
   * meta-keys — the eager `:ormap` existence marker (written at the first durable
   * OR write, so add-only stores are included) OR the legacy `:tombstones` key.
   * Snapshot semantics are mandatory (see startMerkleSync): a racily/lazily
   * discovered non-empty store must never be missing from the FIRST snapshot
   * taken this connection, or the exact cross-map ACK-inflation gap reopens.
   *
   * Runs the one-time legacy-store backfill FIRST (idempotent, gated by a durable
   * done-flag) so a pre-fix add-only store — which has neither marker — is stamped
   * with a `:ormap` marker before enumeration reads it. Backfill inside the snapshot
   * computation (not a separate suppress-until-migrated flag) makes held-set
   * incompleteness impossible at EVERY instant: either the backfill has completed
   * and every persisted OR-Map carries a marker, or it throws and this connection
   * fail-closes below.
   *
   * THROWS on enumeration/backfill failure — deliberately fail-closed. Swallowing the
   * error and returning the in-memory-maps-only subset would hand the
   * min-barrier a PARTIAL held-set: a persisted store the snapshot silently
   * missed could then hold un-received tombstones while the device's ACK
   * advances past them — the exact inflation hole this snapshot exists to
   * close. The caller (startMerkleSync) converts the throw into a
   * connection-wide ACK suppression (heldSetIncomplete) instead.
   */
  private async computeHeldOrMapNames(): Promise<Set<string>> {
    await this.backfillLegacyOrMapMarkers();
    const held = new Set<string>();
    for (const [mapName, map] of this.maps) {
      if (map instanceof ORMap) {
        held.add(mapName);
      }
    }
    const metaKeys = await this.storageAdapter.getAllMetaKeys();
    for (const key of metaKeys) {
      const match = HELD_ORMAP_META_RE.exec(key);
      if (match) {
        held.add(match[1]);
      }
    }
    return held;
  }

  /**
   * One-time migration for stores persisted BEFORE the eager `:ormap` marker
   * existed. A pre-fix add-only OR-Map has KV records but no `:ormap` marker and
   * (being add-only) no `:tombstones` key either, so it is invisible to the
   * held-set snapshot — the cross-map ACK-inflation vector for legacy data. This
   * scans the KV keyspace once, attributes every OR-Map by data shape, and stamps
   * its marker so subsequent snapshots discover it in O(1).
   *
   * Correctness notes:
   * - Scans ALL keys (never a sample): a missed add-only map with a single key is
   *   exactly the bug. A name is classified OR the moment ANY key under its prefix
   *   holds an ARRAY (the ORMap records-array shape; an LWWRecord is always a single
   *   object, never a bare array), so a `:`-prefix collision (TODO-577) between the
   *   FIRST-colon-split name and an LWW *value* only ever ADDS a phantom OR name
   *   (over-conservative: coverage 0 until it syncs), never hides a real one.
   * - KNOWN GAP (TODO-577, latent until the durability watermark activates): the
   *   first-colon split (`indexOf(':')`) means a map whose NAME contains a colon
   *   (`"a:b"`, data key `"a:b:k"`) is attributed to `"a"`, so its own `:ormap`
   *   marker is never stamped. If a sibling `"a"` also exists and syncs while `"a:b"`
   *   is never re-opened this session, the min-barrier CAN advance past `"a:b"`'s
   *   un-received tombstones — i.e. this colon-in-NAME case CAN hide a real map,
   *   unlike the colon-in-value case above. Legacy-backfill-only (every post-fix map
   *   stamps its marker under its full name; `HELD_ORMAP_META_RE`'s greedy capture
   *   recovers colon names correctly). Closed by forbidding `:` in map names or an
   *   injective key scheme — see TODO-577.
   * - Gated by a durable done-flag set ONLY on success. A throw propagates to
   *   `computeHeldOrMapNames` → the connection fail-closes (heldSetIncomplete) and
   *   the scan is retried on the next connection. This makes the upgrade path safe
   *   even though the server cursor keyspace was reset to `_v2` (a fresh cursor has
   *   no conservative fallback, so an unmarked legacy add-only map would otherwise
   *   let the device build inflated ACKs on first prune).
   */
  private async backfillLegacyOrMapMarkers(): Promise<void> {
    if (await this.storageAdapter.getMeta(ORMAP_BACKFILL_DONE_KEY)) return;

    const keys = await this.storageAdapter.getAllKeys();
    const orNames = new Set<string>();
    for (const fullKey of keys) {
      const ci = fullKey.indexOf(':');
      if (ci <= 0) continue;
      const name = fullKey.substring(0, ci);
      if (orNames.has(name)) continue; // already attributed OR — skip redundant reads
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- shape is discriminated by Array.isArray below; the OR-Map records-array vs LWWRecord-object distinction is all we need
      const value = await this.storageAdapter.get<any>(fullKey);
      if (Array.isArray(value)) {
        orNames.add(name);
      }
    }

    for (const name of orNames) {
      await this.storageAdapter.setMeta(orMapMarkerKey(name), 1);
      this.markedOrMaps.add(name);
    }
    // Set the done-flag LAST — only a fully-successful scan is recorded, so any
    // failure above leaves it unset and the migration retries next connection.
    await this.storageAdapter.setMeta(ORMAP_BACKFILL_DONE_KEY, true);
    if (orNames.size > 0) {
      logger.info(
        { orMapNames: Array.from(orNames) },
        'Backfilled OR-Map existence markers for legacy persisted stores',
      );
    }
  }

  /**
   * Instantiate + restore a persisted-but-not-yet-opened OR-Map by name so
   * `startMerkleSync` can sync it even though the application has not called
   * `getORMap()` on it this session. Registers the instance in `this.maps`
   * (mirrors `TopGunClient.getORMap`'s restore path via `registerMap`) so a
   * later `getORMap()` call finds live, already-synced state. Liveness fix:
   * without this, an abandoned local store would permanently stall the
   * covering-epoch min-barrier for every OTHER held map on this device.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap generic params are erased at the registry level; restored instance is later re-typed by TopGunClient.getORMap's overload
  private async instantiateAndRestoreOrMap(mapName: string): Promise<ORMap<any, any>> {
    const existing = this.maps.get(mapName);
    if (existing instanceof ORMap) {
      return existing;
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap generic params are erased at the registry level; actual types are supplied by the caller's later getORMap() overload
    const map = new ORMap<any, any>(this.hlc);
    this.registerMap(mapName, map);

    try {
      const tombstoneKey = `__sys__:${mapName}:tombstones`;
      const tombstones = await this.storageAdapter.getMeta(tombstoneKey);
      if (Array.isArray(tombstones)) {
        for (const tag of tombstones) {
          map.applyTombstone(tag);
        }
      }

      const keys = await this.storageAdapter.getAllKeys();
      const prefix = `${mapName}:`;
      for (const fullKey of keys) {
        if (!fullKey.startsWith(prefix)) continue;
        const keyPart = fullKey.substring(prefix.length);
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- restored KV value shape is validated by the Array.isArray guard below; ORMapRecord value type is erased at the storage layer
        const data = await this.storageAdapter.get<any>(fullKey);
        if (Array.isArray(data)) {
          for (const record of data as ORMapRecord<unknown>[]) {
            map.apply(keyPart, record);
          }
        }
      }
    } catch (err) {
      logger.error(
        { mapName, err },
        'Failed to restore a persisted-but-not-instantiated ORMap for covering-epoch sync',
      );
    }

    return map;
  }

  /**
   * Record that `mapName`'s OR-Map data is durably applied through `epoch` on
   * this connection, then re-derive the device-wide confirmed-apply cursor as
   * the MIN coverage across every OR-Map this device holds (the held-set
   * snapshot from startMerkleSync). A held map with no completed sync this
   * connection contributes 0, so the MIN stalls at 0 until every held map has
   * synced at least once — this is the fix for the cross-map covering-epoch
   * ACK-inflation bug: a single map's sync completion can no longer license an
   * ACK that outruns another held map's actual delivery.
   */
  private applyMapCoverage(mapName: string, epoch: number): void {
    if (!(epoch > 0)) return;
    const current = this.orMapCoverage.get(mapName) ?? 0;
    if (epoch > current) {
      this.orMapCoverage.set(mapName, epoch);
    }
    // NOTE: a non-advancing epoch still falls through to the min/emit below —
    // emitConfirmedApply advances lastAckedEpoch ONLY when the ACK frame was
    // actually sent, so a previously-FAILED send (socket unavailable) must be
    // re-attempted the next time the same coverage is re-proven, or that epoch's
    // ACK would be dropped forever (the cursor is cumulative-monotonic).

    if (this.heldSetIncomplete) {
      // Fail-closed: this connection's held-set enumeration failed, so the
      // barrier's universe is unknown — a min over a PARTIAL set could advance
      // the ACK past a persisted store's un-received tombstones (the exact
      // inflation hole). Suppress every ACK until a later connection snapshots
      // successfully; the server cursor is monotone, keeping its old value
      // (correct, conservative).
      if (!this.heldSetIncompleteWarned) {
        this.heldSetIncompleteWarned = true;
        logger.warn(
          { mapName, epoch },
          'ORMap covering-epoch ACK suppressed for this connection: held-set enumeration failed (fail-closed)',
        );
      }
      return;
    }

    if (!this.heldOrMapNames || this.heldOrMapNames.size === 0) {
      // No snapshot yet, or nothing held: an untracked device pins nothing
      // server-side (safe) — never ACK without a held-set to confirm coverage over.
      return;
    }

    let minCoverage = Number.POSITIVE_INFINITY;
    for (const held of this.heldOrMapNames) {
      const coverage = this.orMapCoverage.get(held) ?? 0;
      if (coverage < minCoverage) minCoverage = coverage;
      if (minCoverage <= 0) break;
    }
    if (!Number.isFinite(minCoverage)) minCoverage = 0;

    if (minCoverage <= 0) {
      // At least one held map has not completed a sync on this connection yet —
      // stall the ACK rather than advance past a map we have not heard back
      // from (this is the documented one-slow-map liveness degradation: it
      // self-resolves once that map syncs, or eventually via MaxRetention
      // forgetting an unreachable device).
      logger.warn(
        { mapName, epoch, heldMapCount: this.heldOrMapNames.size },
        'ORMap covering-epoch ACK stalled: at least one held map has not completed sync on this connection',
      );
      return;
    }

    this.emitConfirmedApply(minCoverage);
  }

  public async recordOperation(
    mapName: string,
    opType: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE',
    key: string,
    data: {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op log entries hold records with unknown value type; type is determined by the map the entry belongs to
      record?: LWWRecord<any>;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMapRecord value type erased in the op log
      orRecord?: ORMapRecord<any>;
      orTag?: string;
      timestamp: Timestamp;
    },
    /**
     * KV/meta mutations that must commit atomically with this op (the record/records-array
     * put + ORMap tombstone meta). When provided, the op + mutations land in ONE durable
     * transaction (crash-consistent); otherwise the op is appended alone.
     */
    mutations?: StorageMutation[],
  ): Promise<string> {
    // Check backpressure before adding new operation (delegates to BackpressureController)
    await this.backpressureController.checkBackpressure();

    const opLogEntry: Omit<OpLogEntry, 'id'> & { id?: string } = {
      mapName,
      opType,
      key,
      record: data.record,
      orRecord: data.orRecord,
      orTag: data.orTag,
      timestamp: data.timestamp,
      synced: false,
    };

    // Stamp the eager OR-Map existence marker in the SAME transaction as the first
    // durable OR write for this map (local-write seam). Bundling it into `mutations`
    // makes marker-and-data atomic — no crash window in which the records are durable
    // but the marker (and therefore the held-set membership) is not. Once per session,
    // and the in-memory guard is set only AFTER the commit succeeds (below) so a
    // rejected commit does not leave the map flagged-but-unmarked.
    const stampingOrMapMarker =
      (opType === 'OR_ADD' || opType === 'OR_REMOVE') &&
      mutations !== undefined &&
      mutations.length > 0 &&
      !this.markedOrMaps.has(mapName);
    if (stampingOrMapMarker && mutations) {
      mutations.unshift({ store: 'meta', type: 'put', key: orMapMarkerKey(mapName), value: 1 });
    }

    // Commit-first ordering: persist durably BEFORE touching the in-memory opLog. If the
    // commit rejects, no op is pushed — the in-memory opLog and the durable op_log stay
    // consistent (no op-without-record in either layer) — and we rethrow so the caller's
    // .catch surfaces the durability failure. The atomic commitWrite keeps the (record, op)
    // pair crash-consistent on disk; the bare appendOpLog path is for ops with no KV write.
    const id =
      mutations && mutations.length > 0
        ? // eslint-disable-next-line @typescript-eslint/no-explicit-any -- adapter OpLogEntry (numeric id/synced) differs from the engine's; the adapter stores it opaquely
          await this.storageAdapter.commitWrite(mutations, opLogEntry as any)
        : // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IStorageAdapter.appendOpLog accepts any; the op log entry is typed internally but the adapter interface is backend-agnostic
          await this.storageAdapter.appendOpLog(opLogEntry as any);
    opLogEntry.id = String(id);
    // Commit succeeded and the marker (if any) is now durable — arm the guard so we
    // don't re-stamp it on every subsequent write to this map.
    if (stampingOrMapMarker) {
      this.markedOrMaps.add(mapName);
    }

    this.opLog.push(opLogEntry as OpLogEntry);
    // Notify per-record sync-state tracker on fresh local write.
    this.recordSyncStateTracker.onAppend(opLogEntry as OpLogEntry);

    // Check high water mark after adding operation (delegates to BackpressureController)
    this.backpressureController.checkHighWaterMark();

    if (this.isAuthenticated()) {
      this.syncPendingOperations();
    }

    return opLogEntry.id;
  }

  private syncPendingOperations(): void {
    const pending = this.opLog.filter((op) => !op.synced);
    if (pending.length === 0) return;

    logger.info({ count: pending.length }, 'Syncing pending operations');

    // Delegate to connection provider's sendBatch when available (cluster mode).
    // This allows ClusterClient to group operations by target partition owner
    // and send separate OP_BATCH messages per node.
    const connectionProvider = this.webSocketManager.getConnectionProvider();
    if (connectionProvider.sendBatch) {
      const results = connectionProvider.sendBatch(
        pending.map((op) => ({ key: op.key, message: op })),
      );
      const failedKeys = [...results.entries()]
        .filter(([, success]) => !success)
        .map(([key]) => key);
      if (failedKeys.length > 0) {
        logger.warn(
          { failedKeys, count: failedKeys.length },
          'Some batch operations failed to send',
        );
      }
      return;
    }

    // Fallback: send all ops in a single OP_BATCH (single-server mode)
    this.sendMessage({
      type: 'OP_BATCH',
      payload: {
        ops: pending,
      },
    });
  }

  private async startMerkleSync(): Promise<void> {
    // Snapshot semantics are MANDATORY: fix the held-map set (and therefore the
    // covering-epoch ACK min-barrier) once, before the first sync round-trip on
    // this connection sends anything — no message goes out before this resolves,
    // so no covering-epoch ACK can fire against a stale or partial snapshot. A
    // map discovered later (registerMap) joins with coverage 0 and can only
    // narrow, never retroactively widen, an already-computed barrier.
    this.orMapCoverage = new Map();
    this.heldOrMapNames = null;
    this.heldSetIncomplete = false;
    this.heldSetIncompleteWarned = false;
    try {
      this.heldOrMapNames = await this.computeHeldOrMapNames();
    } catch (err) {
      // Fail-closed: NEVER run the barrier over a partial snapshot. Data sync
      // for the maps this process HAS open still proceeds below (safe — sync
      // only pulls state, the barrier only gates the confirmed-apply ACK);
      // covering-epoch ACKs stay suppressed for the whole connection, so the
      // server's monotone cursor simply keeps its previous value.
      this.heldSetIncomplete = true;
      logger.error(
        { err },
        'ORMap held-set enumeration failed: covering-epoch ACKs disabled for this connection (fail-closed); data sync continues',
      );
    }

    if (this.heldOrMapNames) {
      for (const mapName of this.heldOrMapNames) {
        this.orMapCoverage.set(mapName, 0);
      }

      // Instantiate + restore every persisted-but-not-yet-opened OR-Map so it can
      // be synced (liveness — otherwise an abandoned local store permanently
      // stalls the min-barrier for every OTHER held map on this device).
      for (const mapName of this.heldOrMapNames) {
        if (!(this.maps.get(mapName) instanceof ORMap)) {
          await this.instantiateAndRestoreOrMap(mapName);
        }
      }
    }

    for (const [mapName, map] of this.maps) {
      if (map instanceof LWWMap) {
        this.merkleSyncHandler.sendSyncInit(mapName, this.lastSyncTimestamp);
      } else if (map instanceof ORMap) {
        this.orMapSyncHandler.sendSyncInit(mapName, this.lastSyncTimestamp);
      }
    }
  }

  public setAuthToken(token: string): void {
    this.authToken = token;
    this.tokenProvider = null;
    // A new token may name a different principal, whose server-side per-device cursor
    // is independent. Reset the local confirmed-apply high-water-mark so ACKs for the
    // new identity are not suppressed by the prior identity's epoch (server-side
    // monotone-max makes a redundant re-ACK under the same principal a harmless no-op).
    this.lastAckedEpoch = 0;

    const state = this.stateMachine.getState();
    if (state === SyncState.AUTHENTICATING) {
      // Already connected and waiting for token — send it now
      this.sendAuth();
    } else if (state === SyncState.BACKOFF || state === SyncState.DISCONNECTED) {
      // Force immediate reconnect if we were waiting for retry timer
      logger.info('Auth token set during backoff/disconnect. Reconnecting immediately.');
      this.webSocketManager.clearReconnectTimer();
      // Reset backoff since user provided new credentials
      this.webSocketManager.resetBackoff();
      this.webSocketManager.connect();
    }
  }

  /**
   * Resolve the token currently in effect: the one set directly via
   * {@link setAuthToken}, else the value produced by the configured token
   * provider, else `null`. Does not mutate connection state — it is a read used
   * by callers that need to authenticate a side-channel request with the same
   * credentials the WebSocket uses.
   *
   * If a token provider is configured and rejects, this rejects too: a provider
   * failure (OAuth down, expired refresh) is distinct from "no credentials
   * configured" (null) and callers must be able to tell them apart rather than
   * silently degrade to an unauthenticated request.
   */
  public async getResolvedToken(): Promise<string | null> {
    if (this.authToken) return this.authToken;
    if (this.tokenProvider) {
      return await this.tokenProvider();
    }
    return null;
  }

  public setTokenProvider(provider: () => Promise<string | null>): void {
    this.tokenProvider = provider;
    const state = this.stateMachine.getState();
    if (state === SyncState.AUTHENTICATING) {
      this.sendAuth();
    }
  }

  private async sendAuth(): Promise<void> {
    if (this.tokenProvider) {
      try {
        const token = await this.tokenProvider();
        if (token) {
          this.authToken = token;
        }
      } catch (err) {
        logger.error({ err }, 'Failed to get token from provider');
        return;
      }
    }

    // Ensure the persisted device credential is loaded before presenting it, so a
    // cold restart rebinds the same identity even if the socket opened before the
    // boot-time load resolved. Shares the single memoized read with loadOpLog.
    await this.ensureDeviceCredentialLoaded();

    const token = this.authToken;

    // No token to present. Two cases, both "do not send an AUTH frame":
    //   - JWT-with-provider that yielded nothing → park in AUTHENTICATING (preserves the
    //     contract that resolved the earlier no-token deadlock; onAuthRequired surfaces it).
    //   - Token-less/NO_AUTH → device identity is presented on a dedicated DEVICE_HELLO
    //     (see handleConnectionEstablished), never an empty-token AUTH (a real JWT server
    //     would AUTH_FAIL + disconnect it).
    if (!token) return;

    // Credentialed AUTH: bundle the persisted deviceToken so the server present-or-mints
    // the device identity in the same round-trip and returns it on AUTH_ACK.
    const authMessage: AuthMessage = { type: 'AUTH', token };
    if (this.deviceToken) {
      authMessage.deviceToken = this.deviceToken;
    }
    this.sendMessage(authMessage);
  }

  /**
   * Present the device credential on a dedicated DEVICE_HELLO frame (token-less path).
   * Orthogonal to AUTH so a JWT server silently drops it (Phase-1 non-AUTH) rather than
   * tearing the connection down. Awaits the persisted-credential load so a cold restart
   * re-presents the same identity.
   */
  private async sendDeviceHello(): Promise<void> {
    await this.ensureDeviceCredentialLoaded();
    const hello: DeviceHelloMessage = { type: 'DEVICE_HELLO' };
    if (this.deviceToken) {
      hello.deviceToken = this.deviceToken;
    }
    this.sendMessage(hello);
  }

  /**
   * DEVICE_ACK received: persist the server-issued device identity and complete the
   * connection auth-optional (NO_AUTH stays unauthenticated — no JWT principal).
   */
  private handleDeviceAck(message: DeviceAckMessage): void {
    logger.info('Device identity acknowledged');
    // Always persist the server-issued identity — it is ours to keep even if this
    // DEVICE_ACK arrives late (after the grace timer already completed the connection).
    this.persistDeviceCredential(message);
    // If the opportunistic device-ack wait has already ended (grace timer fired,
    // AUTH_REQUIRED arrived, or a real AUTH_ACK landed), the connection is past
    // AUTHENTICATING — a late DEVICE_ACK must not re-drive the auth state machine
    // (which would spuriously bounce CONNECTED → SYNCING and re-run post-auth wiring).
    if (!this.deviceAckPending) {
      return;
    }
    this.deviceAckPending = false;
    if (this.authRequiredGraceTimer) {
      clearTimeout(this.authRequiredGraceTimer);
      this.authRequiredGraceTimer = null;
    }
    // DEVICE_ACK does not authenticate a principal; reuse the SYNCING → CONNECTED wiring.
    this.handleAuthAck();
  }

  /**
   * Subscribe to a standard query.
   * Delegates to QueryManager.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter is erased at the SyncEngine routing level; typed handles are created in TopGunClient overloads
  public subscribeToQuery(query: QueryHandle<any>): void {
    this.queryManager.subscribeToQuery(query);
  }

  /**
   * Subscribe to a topic.
   * Delegates to TopicManager.
   */
  public subscribeToTopic(topic: string, handle: TopicHandle): void {
    this.topicManager.subscribeToTopic(topic, handle);
  }

  /**
   * Unsubscribe from a topic.
   * Delegates to TopicManager.
   */
  public unsubscribeFromTopic(topic: string): void {
    this.topicManager.unsubscribeFromTopic(topic);
  }

  /**
   * Publish a message to a topic.
   * Delegates to TopicManager.
   */
  public publishTopic(topic: string, data: unknown): void {
    this.topicManager.publishTopic(topic, data);
  }

  /**
   * Get topic queue status.
   * Delegates to TopicManager.
   */
  public getTopicQueueStatus(): { size: number; maxSize: number } {
    return this.topicManager.getTopicQueueStatus();
  }

  /**
   * Executes a query against local storage immediately.
   * Delegates to QueryManager.
   */
  public async runLocalQuery(
    mapName: string,
    filter: QueryFilter,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- local query returns raw record values whose type is unknown at the SyncEngine level; callers cast to T
  ): Promise<{ key: string; value: any }[]> {
    return this.queryManager.runLocalQuery(mapName, filter);
  }

  /**
   * Unsubscribe from a query.
   * Delegates to QueryManager.
   */
  public unsubscribeFromQuery(queryId: string): void {
    this.queryManager.unsubscribeFromQuery(queryId);
  }

  /**
   * Request a distributed lock.
   * Delegates to LockManager.
   */
  public requestLock(
    name: string,
    requestId: string,
    ttl: number,
  ): Promise<{ fencingToken: number }> {
    return this.lockManager.requestLock(name, requestId, ttl);
  }

  /**
   * Release a distributed lock.
   * Delegates to LockManager.
   */
  public releaseLock(name: string, requestId: string, fencingToken: number): Promise<boolean> {
    return this.lockManager.releaseLock(name, requestId, fencingToken);
  }

  private async handleServerMessage(message: {
    type: string;
    payload?: unknown;
    timestamp?: Timestamp;
  }): Promise<void> {
    // Emit to generic listeners (used by EventJournalReader)
    this.emitMessage(message);

    // Message-first legacy inference: while a token-less DEVICE_HELLO awaits its
    // DEVICE_ACK, any server message that is not DEVICE_ACK (nor AUTH_REQUIRED, which
    // has its own handler and means "this is a JWT server") indicates a server that
    // predates device identity — proceed auth-optional immediately instead of waiting
    // out the grace timeout.
    if (
      this.deviceAckPending &&
      message.type !== 'DEVICE_ACK' &&
      message.type !== 'AUTH_REQUIRED'
    ) {
      this.deviceAckPending = false;
      this.completeAuthOptionalConnection();
      // Fall through so the triggering message is still routed normally.
    }

    // Handle BATCH specially (recursive unbatch)
    if (message.type === 'BATCH') {
      await this.handleBatch(message as BatchMessage);
      return;
    }

    // Route to registered handler
    await this.messageRouter.route(message);

    // Update HLC if message has an HLC Timestamp struct (millis + counter + nodeId).
    // Some messages (e.g. PONG) have a raw numeric `timestamp` field — passing that
    // to HLC.update() would poison the clock with NaN via Number(undefined).
    const ts = message.timestamp;
    if (ts && typeof ts === 'object' && 'millis' in ts && 'counter' in ts && 'nodeId' in ts) {
      this.hlc.update(ts);
      this.lastSyncTimestamp = Number(ts.millis);
      await this.saveOpLog();
    }
  }

  // ============================================
  // Message Handler Helpers (extracted from switch)
  // ============================================

  private async handleBatch(message: BatchMessage): Promise<void> {
    // Unbatch and process each message
    // Format: [4 bytes: count][4 bytes: len1][msg1][4 bytes: len2][msg2]...
    const batchData = message.data;
    const view = new DataView(batchData.buffer, batchData.byteOffset, batchData.byteLength);
    let offset = 0;

    const count = view.getUint32(offset, true);
    offset += 4;

    for (let i = 0; i < count; i++) {
      const msgLen = view.getUint32(offset, true);
      offset += 4;

      const msgData = batchData.slice(offset, offset + msgLen);
      offset += msgLen;

      const innerMsg = deserialize(msgData) as {
        type: string;
        payload?: unknown;
        timestamp?: Timestamp;
      };
      await this.handleServerMessage(innerMsg);
    }
  }

  private handleAuthAck(message?: AuthAckMessage): void {
    logger.info('Authenticated successfully');

    // A real AUTH_ACK resolves the opportunistic device handshake — cancel the
    // legacy grace timer and clear the pending flag so it cannot fire later.
    this.deviceAckPending = false;
    if (this.authRequiredGraceTimer) {
      clearTimeout(this.authRequiredGraceTimer);
      this.authRequiredGraceTimer = null;
    }

    // Persist any server-issued device credential carried on the ack. Absent
    // deviceToken (plain re-bind) keeps the existing token.
    if (message) {
      this.persistDeviceCredential(message);
    }

    const wasAuthenticated = this.isAuthenticated();

    // Transition to SYNCING state
    this.stateMachine.transition(SyncState.SYNCING);

    // Reset backoff on successful auth
    this.webSocketManager.resetBackoff();

    this.syncPendingOperations();

    // Flush any queued topic messages from offline period (delegates to TopicManager)
    this.topicManager.flushTopicQueue();

    // Only re-subscribe on first authentication to prevent UI flickering
    if (!wasAuthenticated) {
      this.webSocketManager.startHeartbeat();
      // Fire-and-forget: the held-map snapshot + persisted-store restore are
      // async, but nothing here depends on their completion — sendSyncInit
      // messages go out once the snapshot resolves, still well before any
      // response (and therefore any covering-epoch ACK) could arrive.
      this.startMerkleSync().catch((err) => logger.error({ err }, 'startMerkleSync failed'));
      // Re-subscribe all queries via QueryManager
      this.queryManager.resubscribeAll();
      // Re-subscribe topics via TopicManager
      this.topicManager.resubscribeAll();
    }

    // After initial sync setup, transition to CONNECTED
    // In a real implementation, you might wait for SYNC_COMPLETE message
    this.stateMachine.transition(SyncState.CONNECTED);
  }

  private handleAuthFail(message: AuthFailMessage): void {
    logger.error({ error: message.error }, 'Authentication failed');
    this.authToken = null; // Clear invalid token
    // Stay in AUTHENTICATING or go to ERROR depending on severity
    // For now, let the connection close naturally or retry with new token
  }

  private handleOpAck(message: OpAckMessage): void {
    const { lastId, achievedLevel, results } = message.payload;
    logger.info({ lastId, achievedLevel, hasResults: !!results }, 'Received ACK for ops');

    // Handle per-operation results if available
    if (results && Array.isArray(results)) {
      for (const result of results) {
        const op = this.opLog.find((o) => o.id === result.opId);
        if (op && !op.synced) {
          op.synced = true;
          logger.debug(
            { opId: result.opId, achievedLevel: result.achievedLevel, success: result.success },
            'Op ACK with Write Concern',
          );
          // Notify per-record sync-state tracker that this op flipped synced=true.
          this.recordSyncStateTracker.onAcknowledge(op);
        }
        // Resolve pending Write Concern promise if exists (delegates to WriteConcernManager)
        this.writeConcernManager.resolveWriteConcernPromise(result.opId, result);
      }
    }

    // Mark all ops up to lastId as synced (numeric comparison — IDs are stringified integers)
    const lastIdNum = parseInt(lastId, 10);
    let maxSyncedId = -1;
    let ackedCount = 0;

    if (!isNaN(lastIdNum)) {
      // Normal path: server returned a valid numeric lastId
      this.opLog.forEach((op) => {
        if (op.id) {
          const opIdNum = parseInt(op.id, 10);
          if (!isNaN(opIdNum) && opIdNum <= lastIdNum) {
            if (!op.synced) {
              ackedCount++;
              // Per-record sync-state tracker — emit only on the actual flip.
              op.synced = true;
              this.recordSyncStateTracker.onAcknowledge(op);
            } else {
              op.synced = true;
            }
            if (opIdNum > maxSyncedId) {
              maxSyncedId = opIdNum;
            }
          }
        }
      });
    } else {
      // Fallback: server returned non-numeric lastId (e.g. "unknown", "undefined").
      // The server ACKed the batch, so mark ALL pending ops as synced.
      logger.warn({ lastId }, 'OP_ACK has non-numeric lastId — marking all pending ops as synced');
      this.opLog.forEach((op) => {
        if (!op.synced) {
          ackedCount++;
          op.synced = true;
          // Per-record sync-state tracker — emit only on the actual flip.
          this.recordSyncStateTracker.onAcknowledge(op);
          const opIdNum = parseInt(op.id, 10);
          if (!isNaN(opIdNum) && opIdNum > maxSyncedId) {
            maxSyncedId = opIdNum;
          }
        }
      });
    }

    if (maxSyncedId !== -1) {
      this.storageAdapter
        .markOpsSynced(maxSyncedId)
        .catch((err) => logger.error({ err }, 'Failed to mark ops synced'));
    }

    // Compaction (in-memory): splice acked ops out of opLog so it holds only pending ops.
    // Without this the array grows for the whole session and getPendingOpsCount() scans O(n)
    // on every write. The tracker was already notified above; the durable KV record is the
    // source of truth, so a synced op has no further in-memory use.
    if (ackedCount > 0 || maxSyncedId !== -1) {
      for (let i = this.opLog.length - 1; i >= 0; i--) {
        if (this.opLog[i].synced) {
          this.opLog.splice(i, 1);
        }
      }
    }

    // Check low water mark after ACKs reduce pending count (delegates to BackpressureController)
    if (ackedCount > 0) {
      this.backpressureController.checkLowWaterMark();
    }
  }

  private handleQueryResp(message: QueryRespMessage): void {
    const { queryId, results, nextCursor, hasMore, cursorStatus, merkleRootHash } = message.payload;
    const query = this.queryManager.getQueries().get(queryId);
    if (query) {
      query.onResult(results, 'server', merkleRootHash);
      query.updatePaginationInfo({ nextCursor, hasMore, cursorStatus });
    }
  }

  private handleQueryUpdate(message: QueryUpdateMessage): void {
    const { queryId, key, value, changeType } = message.payload;
    const query = this.queryManager.getQueries().get(queryId);
    if (query) {
      query.onUpdate(key, changeType === 'LEAVE' ? null : value);
    }
  }

  private async handleServerEvent(message: ServerEventMessage): Promise<void> {
    // Modified to support ORMap
    const { mapName, eventType, key, record, orRecord, orTag } = message.payload;
    await this.applyServerEvent(mapName, eventType, key, record, orRecord, orTag);
    // Apply-not-receive: coverage is reported ONLY after applyServerEvent's durable
    // IndexedDB commit above has resolved — never on receive. Routed through the
    // cross-map min-barrier (NOT emitConfirmedApply directly): a live push proves
    // delivery for THIS map only, so it may advance this map's coverage but must
    // never license a device-wide ACK past another held map's coverage.
    this.applyMapCoverage(mapName, epochOfServerEvent(message.payload));
  }

  private async handleServerBatchEvent(message: ServerBatchEventMessage): Promise<void> {
    // === OPTIMIZATION: Batch event processing ===
    // Server sends multiple events in a single message for efficiency
    const { events } = message.payload;
    for (const event of events) {
      await this.applyServerEvent(
        event.mapName,
        event.eventType,
        event.key,
        event.record,
        event.orRecord,
        event.orTag,
      );
      // Apply-not-receive + cross-map barrier: report each event's epoch as
      // coverage for ITS OWN map, only after that event's awaits above have
      // durably committed. A batch-wide "highest epoch" emitted directly would
      // bypass the min-barrier — one map's event licensing an ACK past another
      // held map's coverage, the exact cross-map inflation bug.
      this.applyMapCoverage(event.mapName, epochOfServerEvent(event));
    }
  }

  /**
   * Emit a confirmed-apply ACK for the highest server epoch this client has durably
   * applied. Per the apply-not-receive contract, callers MUST invoke this ONLY after
   * the covered server batch has been durably committed to IndexedDB.
   *
   * ONLY `applyMapCoverage` may call this method. The confirmed-apply cursor is
   * device-wide while delivery evidence is per-map, so every ACK MUST pass through
   * the cross-map min-barrier — a direct call from any per-map code path (sync
   * response handler, live push event, future wire additions) would let a single
   * map's delivery license an ACK past another held map's coverage, reintroducing
   * the cross-map ACK-inflation resurrection vector the barrier exists to close.
   *
   * The cursor is cumulative-monotonic: a non-advancing (or zero) epoch is never sent.
   */
  private emitConfirmedApply(appliedEpoch: number): void {
    if (appliedEpoch <= this.lastAckedEpoch) return;
    // Advance the local high-water-mark ONLY if the ACK actually went out. sendMessage
    // returns false when the socket cannot take it (disconnected / buffer full); if we
    // advanced regardless, that epoch's ACK would be dropped and never re-sent (the
    // cursor is cumulative-monotonic — a lower epoch is never re-emitted), leaving the
    // server's cursor stuck. A later applied epoch will re-attempt the ACK.
    if (this.sendMessage({ type: 'CLIENT_APPLY_ACK', cursor: appliedEpoch })) {
      this.lastAckedEpoch = appliedEpoch;
    }
  }

  private async handleGcPrune(message: GcPruneMessage): Promise<void> {
    const { olderThan } = message.payload;
    logger.info({ olderThan: olderThan.millis }, 'Received GC_PRUNE request');

    for (const [name, map] of this.maps) {
      if (map instanceof LWWMap) {
        const removedKeys = map.prune(olderThan);
        for (const key of removedKeys) {
          await this.storageAdapter.remove(`${name}:${key}`);
        }
        if (removedKeys.length > 0) {
          logger.info(
            { mapName: name, count: removedKeys.length },
            'Pruned tombstones from LWWMap',
          );
        }
      } else if (map instanceof ORMap) {
        const removedTags = map.prune(olderThan);
        if (removedTags.length > 0) {
          logger.info({ mapName: name, count: removedTags.length }, 'Pruned tombstones from ORMap');
        }
      }
    }
  }

  public getHLC(): HLC {
    return this.hlc;
  }

  /**
   * Helper method to apply a single server event to the local map.
   * Used by both SERVER_EVENT and SERVER_BATCH_EVENT handlers.
   */
  private async applyServerEvent(
    mapName: string,
    eventType: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE',
    key: string,
    record?: LWWRecord<unknown>,
    orRecord?: ORMapRecord<unknown>,
    orTag?: string,
  ): Promise<void> {
    const localMap = this.maps.get(mapName);
    if (localMap) {
      if (localMap instanceof LWWMap && record) {
        const accepted = localMap.merge(key, record);
        if (accepted) {
          // Server record won LWW: memory now holds it — persist so disk matches.
          await this.storageAdapter.put(`${mapName}:${key}`, record);
        } else if (localMap.adoptServerEcho(key, record)) {
          // Echo of our own optimistic write that the server re-stamped with a
          // timestamp our clock had briefly outrun. The value is unchanged, so
          // adopt the server's authoritative timestamp to converge memory, the
          // Merkle tree, and disk with the server instead of re-requesting
          // buckets forever (the memory/disk HLC skew this used to create).
          await this.storageAdapter.put(`${mapName}:${key}`, record);
        }
        // else: a strictly newer local write supersedes the echo. Keep memory
        // and disk as-is (both carry the local write); do NOT persist the stale
        // server record — that unconditional put was the F8 skew bug.
      } else if (localMap instanceof ORMap) {
        if (eventType === 'OR_ADD' && orRecord) {
          localMap.apply(key, orRecord);
          // Persist server-origin ORMap state so it survives an offline reload — symmetric
          // with the LWW path above. Uses the same storage convention as local ORMap writes.
          await this.persistORMapKey(mapName, key);
        } else if (eventType === 'OR_REMOVE' && orTag) {
          localMap.applyTombstone(orTag);
          // The removed key's record list changed and the tombstone set grew — persist both.
          await this.persistORMapKey(mapName, key);
          await this.persistORMapTombstones(mapName);
        }
      }
    }
  }

  /**
   * Canonical ORMap key persistence. Writes the full record list for `key` under the
   * `mapName:key` convention (or removes the entry when empty). Single source of truth for
   * both local-write (TopGunClient) and server-origin (applyServerEvent / ORMapSyncHandler)
   * ORMap persistence so the two paths cannot diverge.
   */
  public async persistORMapKey(mapName: string, key: string): Promise<void> {
    const map = this.maps.get(mapName);
    if (!(map instanceof ORMap)) return;
    // Marker BEFORE data: the server-origin persist path writes marker and data as
    // two separate storage calls (no transaction here). Writing the marker first
    // means a crash in between leaves the map DISCOVERABLE (marker present, coverage
    // 0, ACKs stalled until it syncs) — over-conservative but safe. The reverse order
    // would leave durable records with no marker, reopening the invisible-store hole.
    await this.ensureOrMapMarker(mapName);
    const records = map.getRecords(key);
    if (records.length > 0) {
      await this.storageAdapter.put(`${mapName}:${key}`, records);
    } else {
      await this.storageAdapter.remove(`${mapName}:${key}`);
    }
  }

  /** Canonical ORMap tombstone persistence (the `__sys__:mapName:tombstones` meta key). */
  public async persistORMapTombstones(mapName: string): Promise<void> {
    const map = this.maps.get(mapName);
    if (!(map instanceof ORMap)) return;
    await this.ensureOrMapMarker(mapName);
    await this.storageAdapter.setMeta(`__sys__:${mapName}:tombstones`, map.getTombstones());
  }

  /**
   * Idempotently write the eager `:ormap` existence marker for `mapName` (once per
   * session). Shared by the server-origin persist helpers; the local-write seam
   * stamps the marker transactionally inside `recordOperation` instead. Keeping BOTH
   * durable-write seams marked is what makes the held-set snapshot complete for the
   * add-only class — the local path does NOT flow through these helpers.
   */
  private async ensureOrMapMarker(mapName: string): Promise<void> {
    if (this.markedOrMaps.has(mapName)) return;
    await this.storageAdapter.setMeta(orMapMarkerKey(mapName), 1);
    this.markedOrMaps.add(mapName);
  }

  /**
   * Closes the WebSocket connection and cleans up resources.
   */
  public close(): void {
    // Cancel any pending grace timer before tearing down — prevents stale
    // timer callbacks from firing after the engine is closed or recreated.
    if (this.authRequiredGraceTimer) {
      clearTimeout(this.authRequiredGraceTimer);
      this.authRequiredGraceTimer = null;
    }

    this.webSocketManager.close();

    // Cancel pending Write Concern promises (delegates to WriteConcernManager)
    this.writeConcernManager.cancelAllWriteConcernPromises(new Error('SyncEngine closed'));

    // Clean up CounterManager
    this.counterManager.close();

    // Clean up EntryProcessorClient
    this.entryProcessorClient.close(new Error('SyncEngine closed'));

    // Clean up SearchClient
    this.searchClient.close(new Error('SyncEngine closed'));

    // Clean up SqlClient
    this.sqlClient.close(new Error('SyncEngine closed'));

    // Clean up VectorSearchClient
    this.vectorSearchClient.close(new Error('SyncEngine closed'));

    // Clean up HybridSearchClient
    this.hybridSearchClient.close(new Error('SyncEngine closed'));

    // Tear down per-record sync-state tracker — disposes its registered
    // state-change + rejection subscriptions and clears internal tables.
    this.recordSyncStateTracker.dispose();

    this.stateMachine.transition(SyncState.DISCONNECTED);
    logger.info('SyncEngine closed');
  }

  /**
   * Reset the state machine and connection.
   * Use after fatal errors to start fresh.
   */
  public resetConnection(): void {
    this.close();
    this.stateMachine.reset();
    this.webSocketManager.reset();
    this.webSocketManager.connect();
  }

  // ============================================
  // Failover Support Methods
  // ============================================

  /**
   * Wait for a partition map update from the connection provider.
   * Used when an operation fails with NOT_OWNER error and needs
   * to wait for an updated partition map before retrying.
   *
   * @param timeoutMs - Maximum time to wait (default: 5000ms)
   * @returns Promise that resolves when partition map is updated or times out
   */
  public waitForPartitionMapUpdate(timeoutMs: number = 5000): Promise<void> {
    return new Promise((resolve) => {
      const timeout = setTimeout(resolve, timeoutMs);
      const connectionProvider = this.webSocketManager.getConnectionProvider();

      const handler = () => {
        clearTimeout(timeout);
        connectionProvider.off('partitionMapUpdated', handler);
        resolve();
      };

      connectionProvider.on('partitionMapUpdated', handler);
    });
  }

  /**
   * Wait for the connection to be available.
   * Used when an operation fails due to connection issues and needs
   * to wait for reconnection before retrying.
   *
   * @param timeoutMs - Maximum time to wait (default: 10000ms)
   * @returns Promise that resolves when connected or rejects on timeout
   */
  public waitForConnection(timeoutMs: number = 10000): Promise<void> {
    return new Promise((resolve, reject) => {
      const connectionProvider = this.webSocketManager.getConnectionProvider();

      // If already connected, resolve immediately
      if (connectionProvider.isConnected()) {
        resolve();
        return;
      }

      const timeout = setTimeout(() => {
        connectionProvider.off('connected', handler);
        reject(new Error('Connection timeout waiting for reconnection'));
      }, timeoutMs);

      const handler = () => {
        clearTimeout(timeout);
        connectionProvider.off('connected', handler);
        resolve();
      };

      connectionProvider.on('connected', handler);
    });
  }

  /**
   * Wait for a specific sync state.
   * Useful for waiting until fully connected and synced.
   *
   * @param targetState - The state to wait for
   * @param timeoutMs - Maximum time to wait (default: 30000ms)
   * @returns Promise that resolves when state is reached or rejects on timeout
   */
  public waitForState(targetState: SyncState, timeoutMs: number = 30000): Promise<void> {
    return new Promise((resolve, reject) => {
      // If already in target state, resolve immediately
      if (this.stateMachine.getState() === targetState) {
        resolve();
        return;
      }

      const timeout = setTimeout(() => {
        unsubscribe();
        reject(new Error(`Timeout waiting for state ${targetState}`));
      }, timeoutMs);

      const unsubscribe = this.stateMachine.onStateChange((event) => {
        if (event.to === targetState) {
          clearTimeout(timeout);
          unsubscribe();
          resolve();
        }
      });
    });
  }

  /**
   * Check if the connection provider is connected.
   * Convenience method for failover logic.
   */
  public isProviderConnected(): boolean {
    return this.webSocketManager.getConnectionProvider().isConnected();
  }

  /**
   * Get the connection provider for direct access.
   * Use with caution - prefer using SyncEngine methods.
   */
  public getConnectionProvider(): IConnectionProvider {
    return this.webSocketManager.getConnectionProvider();
  }

  private async resetMap(mapName: string): Promise<void> {
    const map = this.maps.get(mapName);
    if (map) {
      // Clear memory
      if (map instanceof LWWMap) {
        map.clear();
      } else if (map instanceof ORMap) {
        map.clear();
      }
    }

    // Clear storage
    const allKeys = await this.storageAdapter.getAllKeys();
    const mapKeys = allKeys.filter((k) => k.startsWith(mapName + ':'));
    for (const key of mapKeys) {
      await this.storageAdapter.remove(key);
    }
    // Drop the session guard so a later write re-stamps the durable marker. The
    // reserved OR-Map meta (`:ormap` existence marker + `:tombstones` set) is left
    // in place: the meta interface has no delete primitive (only setMeta, which
    // stores undefined rather than removing the key), and a lingering marker only
    // makes a fully-cleared map a self-healing PHANTOM in the next snapshot —
    // included with coverage 0, then cleared to nothing and its empty sync conveys
    // a covering epoch that advances coverage. Over-conservative, never incorrect,
    // and pre-existing for `:tombstones`. A real meta-delete (`removeMeta`) is
    // tracked as a follow-up rather than paid for with an adapter-wide cascade here.
    this.markedOrMaps.delete(mapName);
    logger.info(
      { mapName, removedStorageCount: mapKeys.length },
      'Reset map: Cleared memory and storage',
    );
  }

  // ============ Heartbeat Methods (delegate to WebSocketManager) ============

  /**
   * Returns the last measured round-trip time in milliseconds.
   * Returns null if no PONG has been received yet.
   */
  public getLastRoundTripTime(): number | null {
    return this.webSocketManager.getLastRoundTripTime();
  }

  /**
   * Returns true if the connection is considered healthy based on heartbeat.
   * A connection is healthy if it's online, authenticated, and has received
   * a PONG within the timeout window.
   */
  public isConnectionHealthy(): boolean {
    return this.webSocketManager.isConnectionHealthy();
  }

  // ============ Backpressure Methods (delegated to BackpressureController) ============

  /**
   * Get the current number of pending (unsynced) operations.
   * Delegates to BackpressureController.
   */
  public getPendingOpsCount(): number {
    return this.backpressureController.getPendingOpsCount();
  }

  /**
   * Wait until a specific local op has been confirmed applied by the server.
   *
   * Resolves `'synced'` once the op identified by `opId` is acknowledged (the
   * server applied it). Acknowledged ops are flipped `synced=true` and then
   * spliced out of the in-memory `opLog` (see {@link handleOpAck}); therefore an
   * op that is **absent** from `opLog` has already been acked — that is treated
   * as `'synced'`, not as "unknown".
   *
   * Resolves `'offline'` the moment the client is not online with the op still
   * pending — so a caller (e.g. the MCP `mutate` tool) can fail fast with an
   * honest "queued locally, not yet durable" instead of blocking for the whole
   * timeout. Resolves `'timeout'` if the op stays pending past `timeoutMs` while
   * online (server slow / unreachable mid-flight).
   *
   * This is the authoritative "did the server take my write?" signal for plain
   * map writes. Per-op Write Concern promises only resolve when the server
   * returns per-op `results`, which the single-server OP_BATCH path does not — so
   * this op-log/ACK projection is the correct primitive for confirming a write.
   */
  public async waitForOpSynced(
    opId: string,
    timeoutMs: number,
  ): Promise<'synced' | 'offline' | 'timeout'> {
    const deadline = Date.now() + timeoutMs;

    // Capture the op object ONCE by id, then poll ITS `synced` flag — do NOT
    // re-find by id each tick. The distinction matters for correctness:
    //
    //   - On ack, handleOpAck flips `synced = true` on this exact object BEFORE
    //     compacting it out of the array, so the captured reference observes the
    //     ack even after the splice.
    //   - When an op leaves opLog UNACKED — backpressure drop-oldest evicting an
    //     unsynced op, or the opLog being cleared and rebuilt from storage on
    //     reconnect (`opLog.length = 0`) — its `synced` flag stays false on this
    //     captured object. So we correctly report timeout/offline, never a false
    //     'synced'. A re-find-by-id would instead see the op ABSENT and wrongly
    //     conclude it was acked — reporting a write as durable that the server
    //     never confirmed.
    //
    // If the op is already absent at this first lookup, it was acked + compacted
    // in the brief gap since recordOperation appended it (the fast-ack path — the
    // only way an op leaves opLog between append and this immediate check, since
    // drop/reset require backpressure buildup or a reconnect that cannot occur in
    // that window). That is a genuine ack ⇒ 'synced'.
    const op = this.opLog.find((o) => o.id === opId);
    if (!op) return 'synced';

    while (op.synced !== true) {
      // Fail fast when offline with the op still pending: it will only sync on a
      // future reconnect, so there is nothing to wait for here.
      if (!this.isOnline()) return 'offline';
      if (Date.now() >= deadline) return 'timeout';
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    return 'synced';
  }

  /**
   * Get the current backpressure status.
   * Delegates to BackpressureController.
   */
  public getBackpressureStatus(): BackpressureStatus {
    return this.backpressureController.getBackpressureStatus();
  }

  /**
   * Returns true if writes are currently paused due to backpressure.
   * Delegates to BackpressureController.
   */
  public isBackpressurePaused(): boolean {
    return this.backpressureController.isBackpressurePaused();
  }

  /**
   * Subscribe to backpressure events. Delegates to BackpressureController.
   * The listener type narrows by event name (see overloads).
   * @returns Unsubscribe function
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
    return this.backpressureController.onBackpressure(event as any, listener as any);
  }

  // ============================================
  // Write Concern Methods
  // ============================================

  /**
   * Register a pending Write Concern promise for an operation.
   * Delegates to WriteConcernManager.
   *
   * @param opId - Operation ID
   * @param timeout - Timeout in ms (default: 5000)
   * @returns Promise that resolves with the Write Concern result
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- write concern result is a raw server ACK payload whose shape depends on write concern level
  public registerWriteConcernPromise(opId: string, timeout: number = 5000): Promise<any> {
    return this.writeConcernManager.registerWriteConcernPromise(opId, timeout);
  }

  // ============================================
  // PN Counter Methods - Delegates to CounterManager
  // ============================================

  /**
   * Subscribe to counter updates from server.
   * Delegates to CounterManager.
   * @param name Counter name
   * @param listener Callback when counter state is updated
   * @returns Unsubscribe function
   */
  public onCounterUpdate(
    name: string,
    listener: (state: { positive: Map<string, number>; negative: Map<string, number> }) => void,
  ): () => void {
    return this.counterManager.onCounterUpdate(name, listener);
  }

  /**
   * Request initial counter state from server.
   * Delegates to CounterManager.
   * @param name Counter name
   */
  public requestCounter(name: string): void {
    this.counterManager.requestCounter(name);
  }

  /**
   * Sync local counter state to server.
   * Delegates to CounterManager.
   * @param name Counter name
   * @param state Counter state to sync
   */
  public syncCounter(
    name: string,
    state: { positive: Map<string, number>; negative: Map<string, number> },
  ): void {
    this.counterManager.syncCounter(name, state);
  }

  // ============================================
  // Entry Processor Methods - Delegates to EntryProcessorClient
  // ============================================

  /**
   * Execute an entry processor on a single key atomically.
   * Delegates to EntryProcessorClient.
   *
   * @param mapName Name of the map
   * @param key Key to process
   * @param processor Processor definition
   * @returns Promise resolving to the processor result
   */
  public async executeOnKey<V, R = V>(
    mapName: string,
    key: string,
    processor: EntryProcessorDef<V, R>,
  ): Promise<EntryProcessorResult<R>> {
    return this.entryProcessorClient.executeOnKey(mapName, key, processor);
  }

  /**
   * Execute an entry processor on multiple keys.
   * Delegates to EntryProcessorClient.
   *
   * @param mapName Name of the map
   * @param keys Keys to process
   * @param processor Processor definition
   * @returns Promise resolving to a map of key -> result
   */
  public async executeOnKeys<V, R = V>(
    mapName: string,
    keys: string[],
    processor: EntryProcessorDef<V, R>,
  ): Promise<Map<string, EntryProcessorResult<R>>> {
    return this.entryProcessorClient.executeOnKeys(mapName, keys, processor);
  }

  // ============================================
  // Event Journal Methods
  // ============================================

  /** Message listeners for journal and other generic messages */
  private messageListeners: Set<(message: unknown) => void> = new Set();

  /**
   * Subscribe to all incoming messages.
   * Used by EventJournalReader to receive journal events.
   *
   * @param event Event type (currently only 'message')
   * @param handler Message handler
   */
  public on(event: 'message', handler: (message: unknown) => void): void {
    if (event === 'message') {
      this.messageListeners.add(handler);
    }
  }

  /**
   * Unsubscribe from incoming messages.
   *
   * @param event Event type (currently only 'message')
   * @param handler Message handler to remove
   */
  public off(event: 'message', handler: (message: unknown) => void): void {
    if (event === 'message') {
      this.messageListeners.delete(handler);
    }
  }

  /**
   * Send a message to the server.
   * Public method for EventJournalReader and other components.
   *
   * @param message Message object to send
   */
  public send(message: unknown): void {
    this.sendMessage(message);
  }

  /**
   * Emit message to all listeners.
   * Called internally when a message is received.
   */
  private emitMessage(message: unknown): void {
    for (const listener of this.messageListeners) {
      try {
        listener(message);
      } catch (e) {
        logger.error({ err: e }, 'Message listener error');
      }
    }
  }

  // ============================================
  // Full-Text Search Methods - Delegates to SearchClient
  // ============================================

  /**
   * Perform a one-shot BM25 search on the server.
   * Delegates to SearchClient.
   *
   * @param mapName Name of the map to search
   * @param query Search query text
   * @param options Search options (limit, minScore, boost)
   * @returns Promise resolving to search results
   */
  public async search<T>(
    mapName: string,
    query: string,
    options?: SearchOptions,
  ): Promise<SearchResult<T>[]> {
    return this.searchClient.search<T>(mapName, query, options);
  }

  // ============================================
  // SQL Query API
  // ============================================

  /**
   * Execute a SQL query on the server via DataFusion.
   * Delegates to SqlClient.
   *
   * @param query SQL query string
   * @returns Promise resolving to { columns, rows }
   */
  public async sql(query: string): Promise<SqlQueryResult> {
    return this.sqlClient.sql(query);
  }

  // ============================================
  // Vector Search API
  // ============================================

  /**
   * Perform an ANN vector search on the server.
   * Delegates to VectorSearchClient.
   *
   * @param mapName Name of the map / HNSW index to search
   * @param queryVector Query vector as Float32Array or number[]
   * @param options Search options (k, efSearch, etc.)
   * @returns Promise resolving to ranked VectorSearchClientResult[]
   */
  public async vectorSearch(
    mapName: string,
    queryVector: Float32Array | number[],
    options?: VectorSearchClientOptions,
  ): Promise<VectorSearchClientResult[]> {
    return this.vectorSearchClient.vectorSearch(mapName, queryVector, options);
  }

  // ============================================
  // Hybrid Search API
  // ============================================

  /**
   * Perform a tri-hybrid search (exact + fullText + semantic via RRF) on the server.
   * Delegates to HybridSearchClient.
   *
   * @param mapName Name of the map to search
   * @param queryText Search query text
   * @param options Search options (methods, k, queryVector, etc.)
   * @returns Promise resolving to ranked HybridSearchClientResult[]
   */
  public async hybridSearch(
    mapName: string,
    queryText: string,
    options?: HybridSearchClientOptions,
  ): Promise<HybridSearchClientResult[]> {
    return this.hybridSearchClient.hybridSearch(mapName, queryText, options);
  }

  // ============================================
  // Conflict Resolver Client
  // ============================================

  /**
   * Get the conflict resolver client for registering custom resolvers
   * and subscribing to merge rejection events.
   */
  public getConflictResolverClient(): ConflictResolverClient {
    return this.conflictResolverClient;
  }

  /**
   * Get the per-record sync-state tracker. Used by QueryHandle to project
   * sync-state snapshots filtered to its result-set keys, and by the
   * useSyncState React hook for ad-hoc per-key reads.
   */
  public getRecordSyncStateTracker(): RecordSyncStateTracker {
    return this.recordSyncStateTracker;
  }

  // ============================================
  // Hybrid Query Support - Delegates to QueryManager
  // ============================================

  /**
   * Subscribe to a hybrid query (FTS + filter combination).
   * Delegates to QueryManager.
   */
  public subscribeToHybridQuery<T>(query: HybridQueryHandle<T>): void {
    this.queryManager.subscribeToHybridQuery(query);
  }

  /**
   * Unsubscribe from a hybrid query.
   * Delegates to QueryManager.
   */
  public unsubscribeFromHybridQuery(queryId: string): void {
    this.queryManager.unsubscribeFromHybridQuery(queryId);
  }

  /**
   * Run a local hybrid query (FTS + filter combination).
   * Delegates to QueryManager.
   */
  public async runLocalHybridQuery<T>(
    mapName: string,
    filter: HybridQueryFilter,
  ): Promise<Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>> {
    return this.queryManager.runLocalHybridQuery<T>(mapName, filter);
  }

  /**
   * Handle operation rejected by server (permission denied, validation failure, etc.).
   */
  private handleOpRejected(message: OpRejectedMessage): void {
    const { opId, reason, code } = message.payload;
    logger.warn({ opId, reason, code }, 'Operation rejected by server');

    // Reject pending write concern promise if exists
    this.writeConcernManager.resolveWriteConcernPromise(opId, {
      opId,
      success: false,
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- string literal cast to satisfy the WriteConcernValue type at a rejection code path without importing the enum
      achievedLevel: 'FIRE_AND_FORGET' as any,
      error: reason,
    });
  }

  /**
   * Handle generic error message from server.
   */
  private handleError(message: ErrorMessage): void {
    const { code, message: errorMessage, details } = message.payload;
    logger.error({ code, message: errorMessage, details }, 'Server error received');
  }
}
