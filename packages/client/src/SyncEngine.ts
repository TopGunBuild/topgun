import { HLC, LWWMap, ORMap, serialize, deserialize, evaluatePredicate } from '@topgunbuild/core';
import type { EntryProcessorDef, EntryProcessorResult, EntryProcessKeyResult, SearchOptions, SearchRespPayload } from '@topgunbuild/core';
import type { LWWRecord, ORMapRecord, Timestamp } from '@topgunbuild/core';
import type { IStorageAdapter } from './IStorageAdapter';
import { QueryHandle } from './QueryHandle';
import type { QueryFilter } from './QueryHandle';
import { TopicHandle } from './TopicHandle';
import { logger } from './utils/logger';
import { SyncStateMachine, StateChangeEvent } from './SyncStateMachine';
import { SyncState } from './SyncState';
import { BackpressureError } from './errors/BackpressureError';
import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureStrategy,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from './BackpressureConfig';
import { DEFAULT_BACKPRESSURE_CONFIG } from './BackpressureConfig';
import type { IConnectionProvider } from './types';
import { SingleServerProvider } from './connection/SingleServerProvider';
import { ConflictResolverClient } from './ConflictResolverClient';

/**
 * Search result item from server.
 */
export interface SearchResult<T> {
  key: string;
  value: T;
  score: number;
  matchedTerms: string[];
}

export interface OpLogEntry {
  id: string; // Unique ID for the operation
  mapName: string;
  opType: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE';
  key: string;
  record?: LWWRecord<any>; // LWW Put/Remove (Remove has null value)
  orRecord?: ORMapRecord<any>; // ORMap Add
  orTag?: string; // ORMap Remove (Tombstone tag)
  timestamp: Timestamp; // HLC timestamp of the operation
  synced: boolean; // True if this operation has been successfully pushed to the server
}

export interface HeartbeatConfig {
  intervalMs: number;      // Default: 5000 (5 seconds)
  timeoutMs: number;       // Default: 15000 (15 seconds)
  enabled: boolean;        // Default: true
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
  /** Maximum number of retry attempts before entering ERROR state (default: 10) */
  maxRetries: number;
}

export interface SyncEngineConfig {
  nodeId: string;
  /** @deprecated Use connectionProvider instead */
  serverUrl?: string;
  /** Connection provider (preferred over serverUrl) */
  connectionProvider?: IConnectionProvider;
  storageAdapter: IStorageAdapter;
  reconnectInterval?: number;
  heartbeat?: Partial<HeartbeatConfig>;
  backoff?: Partial<BackoffConfig>;
  backpressure?: Partial<BackpressureConfig>;
}

const DEFAULT_BACKOFF_CONFIG: BackoffConfig = {
  initialDelayMs: 1000,
  maxDelayMs: 30000,
  multiplier: 2,
  jitter: true,
  maxRetries: 10,
};

export class SyncEngine {
  private readonly nodeId: string;
  private readonly serverUrl: string;
  private readonly storageAdapter: IStorageAdapter;
  private readonly hlc: HLC;
  private readonly stateMachine: SyncStateMachine;
  private readonly backoffConfig: BackoffConfig;
  private readonly connectionProvider: IConnectionProvider;
  private readonly useConnectionProvider: boolean;

  private websocket: WebSocket | null = null;
  private opLog: OpLogEntry[] = [];
  private maps: Map<string, LWWMap<any, any> | ORMap<any, any>> = new Map();
  private queries: Map<string, QueryHandle<any>> = new Map();
  private topics: Map<string, TopicHandle> = new Map();
  private pendingLockRequests: Map<string, { resolve: (res: any) => void, reject: (err: any) => void, timer: any }> = new Map();
  private lastSyncTimestamp: number = 0;
  private reconnectTimer: any = null; // NodeJS.Timeout
  private authToken: string | null = null;
  private tokenProvider: (() => Promise<string | null>) | null = null;
  private backoffAttempt: number = 0;

  // Heartbeat state
  private readonly heartbeatConfig: HeartbeatConfig;
  private heartbeatInterval: ReturnType<typeof setInterval> | null = null;
  private lastPongReceived: number = Date.now();
  private lastRoundTripTime: number | null = null;

  // Backpressure state
  private readonly backpressureConfig: BackpressureConfig;
  private backpressurePaused: boolean = false;
  private waitingForCapacity: Array<() => void> = [];
  private highWaterMarkEmitted: boolean = false;
  private backpressureListeners: Map<string, Set<(...args: any[]) => void>> = new Map();

  // Write Concern state (Phase 5.01)
  private pendingWriteConcernPromises: Map<string, {
    resolve: (result: any) => void;
    reject: (error: Error) => void;
    timeoutHandle?: ReturnType<typeof setTimeout>;
  }> = new Map();

  // Conflict Resolver client (Phase 5.05)
  private readonly conflictResolverClient: ConflictResolverClient;

  constructor(config: SyncEngineConfig) {
    // Validate config: either serverUrl or connectionProvider required
    if (!config.serverUrl && !config.connectionProvider) {
      throw new Error('SyncEngine requires either serverUrl or connectionProvider');
    }

    this.nodeId = config.nodeId;
    this.serverUrl = config.serverUrl || '';
    this.storageAdapter = config.storageAdapter;
    this.hlc = new HLC(this.nodeId);

    // Initialize state machine
    this.stateMachine = new SyncStateMachine();

    // Initialize heartbeat config with defaults
    this.heartbeatConfig = {
      intervalMs: config.heartbeat?.intervalMs ?? 5000,
      timeoutMs: config.heartbeat?.timeoutMs ?? 15000,
      enabled: config.heartbeat?.enabled ?? true,
    };

    // Merge backoff config with defaults
    this.backoffConfig = {
      ...DEFAULT_BACKOFF_CONFIG,
      ...config.backoff,
    };

    // Merge backpressure config with defaults
    this.backpressureConfig = {
      ...DEFAULT_BACKPRESSURE_CONFIG,
      ...config.backpressure,
    };

    // Initialize connection provider
    if (config.connectionProvider) {
      this.connectionProvider = config.connectionProvider;
      this.useConnectionProvider = true;
      this.initConnectionProvider();
    } else {
      // Legacy mode: create SingleServerProvider internally
      this.connectionProvider = new SingleServerProvider({ url: config.serverUrl! });
      this.useConnectionProvider = false;
      this.initConnection();
    }

    // Initialize Conflict Resolver client (Phase 5.05)
    this.conflictResolverClient = new ConflictResolverClient(this);

    this.loadOpLog();
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
  // Connection Management
  // ============================================

  /**
   * Initialize connection using IConnectionProvider (Phase 4.5 cluster mode).
   * Sets up event handlers for the connection provider.
   */
  private initConnectionProvider(): void {
    // Transition to CONNECTING state
    this.stateMachine.transition(SyncState.CONNECTING);

    // Set up event handlers
    this.connectionProvider.on('connected', (_nodeId: string) => {
      if (this.authToken || this.tokenProvider) {
        logger.info('ConnectionProvider connected. Sending auth...');
        this.stateMachine.transition(SyncState.AUTHENTICATING);
        this.sendAuth();
      } else {
        logger.info('ConnectionProvider connected. Waiting for auth token...');
        this.stateMachine.transition(SyncState.AUTHENTICATING);
      }
    });

    this.connectionProvider.on('disconnected', (_nodeId: string) => {
      logger.info('ConnectionProvider disconnected.');
      this.stopHeartbeat();
      this.stateMachine.transition(SyncState.DISCONNECTED);
      // Don't schedule reconnect - provider handles it
    });

    this.connectionProvider.on('reconnected', (_nodeId: string) => {
      logger.info('ConnectionProvider reconnected.');
      this.stateMachine.transition(SyncState.CONNECTING);
      if (this.authToken || this.tokenProvider) {
        this.stateMachine.transition(SyncState.AUTHENTICATING);
        this.sendAuth();
      }
    });

    this.connectionProvider.on('message', (_nodeId: string, data: any) => {
      let message: any;
      if (data instanceof ArrayBuffer) {
        message = deserialize(new Uint8Array(data));
      } else if (data instanceof Uint8Array) {
        message = deserialize(data);
      } else {
        try {
          message = typeof data === 'string' ? JSON.parse(data) : data;
        } catch (e) {
          logger.error({ err: e }, 'Failed to parse message from ConnectionProvider');
          return;
        }
      }
      this.handleServerMessage(message);
    });

    this.connectionProvider.on('partitionMapUpdated', () => {
      logger.debug('Partition map updated');
      // Could trigger re-subscriptions if needed
    });

    this.connectionProvider.on('error', (error: Error) => {
      logger.error({ err: error }, 'ConnectionProvider error');
    });

    // Start connection
    this.connectionProvider.connect().catch((err) => {
      logger.error({ err }, 'Failed to connect via ConnectionProvider');
      this.stateMachine.transition(SyncState.DISCONNECTED);
    });
  }

  /**
   * Initialize connection using direct WebSocket (legacy single-server mode).
   */
  private initConnection(): void {
    // Transition to CONNECTING state
    this.stateMachine.transition(SyncState.CONNECTING);

    this.websocket = new WebSocket(this.serverUrl);
    this.websocket.binaryType = 'arraybuffer';

    this.websocket.onopen = () => {
      // WebSocket is open, now we need to authenticate
      // [CHANGE] Don't send auth immediately if we don't have a token
      // This prevents the "AUTH_REQUIRED -> Close -> Retry loop" for anonymous initial connects
      if (this.authToken || this.tokenProvider) {
        logger.info('WebSocket connected. Sending auth...');
        this.stateMachine.transition(SyncState.AUTHENTICATING);
        this.sendAuth();
      } else {
        logger.info('WebSocket connected. Waiting for auth token...');
        // Stay in CONNECTING state until we have a token
        // We're online but not authenticated
        this.stateMachine.transition(SyncState.AUTHENTICATING);
      }
    };

    this.websocket.onmessage = (event) => {
      let message: any;
      if (event.data instanceof ArrayBuffer) {
        message = deserialize(new Uint8Array(event.data));
      } else {
        try {
          message = JSON.parse(event.data);
        } catch (e) {
          logger.error({ err: e }, 'Failed to parse message');
          return;
        }
      }
      this.handleServerMessage(message);
    };

    this.websocket.onclose = () => {
      logger.info('WebSocket disconnected.');
      this.stopHeartbeat();
      this.stateMachine.transition(SyncState.DISCONNECTED);
      this.scheduleReconnect();
    };

    this.websocket.onerror = (error) => {
      logger.error({ err: error }, 'WebSocket error');
      // Error will typically be followed by close, so we don't transition here
    };
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    // Check if we've exceeded max retries
    if (this.backoffAttempt >= this.backoffConfig.maxRetries) {
      logger.error(
        { attempts: this.backoffAttempt },
        'Max reconnection attempts reached. Entering ERROR state.'
      );
      this.stateMachine.transition(SyncState.ERROR);
      return;
    }

    // Transition to BACKOFF state
    this.stateMachine.transition(SyncState.BACKOFF);

    const delay = this.calculateBackoffDelay();
    logger.info({ delay, attempt: this.backoffAttempt }, `Backing off for ${delay}ms`);

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.backoffAttempt++;
      this.initConnection();
    }, delay);
  }

  private calculateBackoffDelay(): number {
    const { initialDelayMs, maxDelayMs, multiplier, jitter } = this.backoffConfig;
    let delay = initialDelayMs * Math.pow(multiplier, this.backoffAttempt);
    delay = Math.min(delay, maxDelayMs);

    if (jitter) {
      // Add jitter: 0.5x to 1.5x of calculated delay
      delay = delay * (0.5 + Math.random());
    }

    return Math.floor(delay);
  }

  /**
   * Reset backoff counter (called on successful connection)
   */
  private resetBackoff(): void {
    this.backoffAttempt = 0;
  }

  /**
   * Send a message through the current connection.
   * Uses connectionProvider if in cluster mode, otherwise uses direct websocket.
   * @param message Message object to serialize and send
   * @param key Optional key for routing (cluster mode only)
   * @returns true if message was sent, false otherwise
   */
  private sendMessage(message: any, key?: string): boolean {
    const data = serialize(message);

    if (this.useConnectionProvider) {
      try {
        this.connectionProvider.send(data, key);
        return true;
      } catch (err) {
        logger.warn({ err }, 'Failed to send via ConnectionProvider');
        return false;
      }
    } else {
      if (this.websocket?.readyState === WebSocket.OPEN) {
        this.websocket.send(data);
        return true;
      }
      return false;
    }
  }

  /**
   * Check if we can send messages (connection is ready).
   */
  private canSend(): boolean {
    if (this.useConnectionProvider) {
      return this.connectionProvider.isConnected();
    }
    return this.websocket?.readyState === WebSocket.OPEN;
  }

  private async loadOpLog(): Promise<void> {
    const storedTimestamp = await this.storageAdapter.getMeta('lastSyncTimestamp');
    if (storedTimestamp) {
      this.lastSyncTimestamp = storedTimestamp;
    }

    const pendingOps = await this.storageAdapter.getPendingOps();
    this.opLog = pendingOps.map(op => ({
      ...op,
      id: String(op.id),
      synced: false
    })) as unknown as OpLogEntry[];

    if (this.opLog.length > 0) {
      logger.info({ count: this.opLog.length }, 'Loaded pending operations from local storage');
    }
  }

  private async saveOpLog(): Promise<void> {
    await this.storageAdapter.setMeta('lastSyncTimestamp', this.lastSyncTimestamp);
  }

  public registerMap(mapName: string, map: LWWMap<any, any> | ORMap<any, any>): void {
    this.maps.set(mapName, map);
  }

  public async recordOperation(
    mapName: string,
    opType: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE',
    key: string,
    data: { record?: LWWRecord<any>; orRecord?: ORMapRecord<any>; orTag?: string; timestamp: Timestamp }
  ): Promise<string> {
    // Check backpressure before adding new operation
    await this.checkBackpressure();

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

    const id = await this.storageAdapter.appendOpLog(opLogEntry as any);
    opLogEntry.id = String(id);

    this.opLog.push(opLogEntry as OpLogEntry);

    // Check high water mark after adding operation
    this.checkHighWaterMark();

    if (this.isAuthenticated()) {
      this.syncPendingOperations();
    }

    return opLogEntry.id;
  }

  private syncPendingOperations(): void {
    const pending = this.opLog.filter(op => !op.synced);
    if (pending.length === 0) return;

    logger.info({ count: pending.length }, 'Syncing pending operations');

    this.sendMessage({
      type: 'OP_BATCH',
      payload: {
        ops: pending
      }
    });
  }

  private startMerkleSync(): void {
    for (const [mapName, map] of this.maps) {
      if (map instanceof LWWMap) {
        logger.info({ mapName }, 'Starting Merkle sync for LWWMap');
        this.sendMessage({
          type: 'SYNC_INIT',
          mapName,
          lastSyncTimestamp: this.lastSyncTimestamp
        });
      } else if (map instanceof ORMap) {
        logger.info({ mapName }, 'Starting Merkle sync for ORMap');
        const tree = map.getMerkleTree();
        const rootHash = tree.getRootHash();

        // Build bucket hashes for all non-empty buckets at depth 0
        const bucketHashes: Record<string, number> = tree.getBuckets('');

        this.sendMessage({
          type: 'ORMAP_SYNC_INIT',
          mapName,
          rootHash,
          bucketHashes,
          lastSyncTimestamp: this.lastSyncTimestamp
        });
      }
    }
  }

  public setAuthToken(token: string): void {
    this.authToken = token;
    this.tokenProvider = null;

    const state = this.stateMachine.getState();
    if (state === SyncState.AUTHENTICATING || state === SyncState.CONNECTING) {
      // If we are already connected (e.g. waiting for token), send it now
      this.sendAuth();
    } else if (state === SyncState.BACKOFF || state === SyncState.DISCONNECTED) {
      // [CHANGE] Force immediate reconnect if we were waiting for retry timer
      logger.info('Auth token set during backoff/disconnect. Reconnecting immediately.');
      if (this.reconnectTimer) {
        clearTimeout(this.reconnectTimer);
        this.reconnectTimer = null;
      }
      // Reset backoff since user provided new credentials
      this.resetBackoff();
      this.initConnection();
    }
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

    const token = this.authToken;
    if (!token) return; // Don't send anonymous auth anymore

    this.sendMessage({
      type: 'AUTH',
      token
    });
  }

  public subscribeToQuery(query: QueryHandle<any>) {
    this.queries.set(query.id, query);
    if (this.isAuthenticated()) {
      this.sendQuerySubscription(query);
    }
  }

  public subscribeToTopic(topic: string, handle: TopicHandle) {
    this.topics.set(topic, handle);
    if (this.isAuthenticated()) {
      this.sendTopicSubscription(topic);
    }
  }

  public unsubscribeFromTopic(topic: string) {
    this.topics.delete(topic);
    if (this.isAuthenticated()) {
      this.sendMessage({
        type: 'TOPIC_UNSUB',
        payload: { topic }
      });
    }
  }

  public publishTopic(topic: string, data: any) {
    if (this.isAuthenticated()) {
      this.sendMessage({
        type: 'TOPIC_PUB',
        payload: { topic, data }
      });
    } else {
      // TODO: Queue topic messages or drop?
      // Spec says Fire-and-Forget, so dropping is acceptable if offline,
      // but queueing is better UX.
      // For now, log warning.
      logger.warn({ topic }, 'Dropped topic publish (offline)');
    }
  }

  private sendTopicSubscription(topic: string) {
    this.sendMessage({
      type: 'TOPIC_SUB',
      payload: { topic }
    });
  }

  /**
   * Executes a query against local storage immediately
   */
  public async runLocalQuery(mapName: string, filter: QueryFilter): Promise<{ key: string, value: any }[]> {
    // Retrieve all keys for the map
    const keys = await this.storageAdapter.getAllKeys();
    const mapKeys = keys.filter(k => k.startsWith(mapName + ':'));

    const results = [];
    for (const fullKey of mapKeys) {
      const record = await this.storageAdapter.get(fullKey);
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
    return results;
  }

  public unsubscribeFromQuery(queryId: string) {
    this.queries.delete(queryId);
    if (this.isAuthenticated()) {
      this.sendMessage({
        type: 'QUERY_UNSUB',
        payload: { queryId }
      });
    }
  }

  private sendQuerySubscription(query: QueryHandle<any>) {
    this.sendMessage({
      type: 'QUERY_SUB',
      payload: {
        queryId: query.id,
        mapName: query.getMapName(),
        query: query.getFilter()
      }
    });
  }

  public requestLock(name: string, requestId: string, ttl: number): Promise<{ fencingToken: number }> {
    if (!this.isAuthenticated()) {
      return Promise.reject(new Error('Not connected or authenticated'));
    }

    return new Promise((resolve, reject) => {
      // Timeout if no response (server might be down or message lost)
      // We set a client-side timeout slightly larger than TTL if TTL is short,
      // but usually we want a separate "Wait Timeout".
      // For now, use a fixed 30s timeout for the *response*.
      const timer = setTimeout(() => {
        if (this.pendingLockRequests.has(requestId)) {
          this.pendingLockRequests.delete(requestId);
          reject(new Error('Lock request timed out waiting for server response'));
        }
      }, 30000);

      this.pendingLockRequests.set(requestId, { resolve, reject, timer });

      try {
        const sent = this.sendMessage({
          type: 'LOCK_REQUEST',
          payload: { requestId, name, ttl }
        });
        if (!sent) {
          clearTimeout(timer);
          this.pendingLockRequests.delete(requestId);
          reject(new Error('Failed to send lock request'));
        }
      } catch (e) {
        clearTimeout(timer);
        this.pendingLockRequests.delete(requestId);
        reject(e);
      }
    });
  }

  public releaseLock(name: string, requestId: string, fencingToken: number): Promise<boolean> {
    if (!this.isOnline()) return Promise.resolve(false);

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pendingLockRequests.has(requestId)) {
          this.pendingLockRequests.delete(requestId);
          // Resolve false on timeout? Or reject?
          // Release is usually fire-and-forget but we wanted ACK.
          resolve(false);
        }
      }, 5000);

      this.pendingLockRequests.set(requestId, { resolve, reject, timer });

      try {
        const sent = this.sendMessage({
          type: 'LOCK_RELEASE',
          payload: { requestId, name, fencingToken }
        });
        if (!sent) {
          clearTimeout(timer);
          this.pendingLockRequests.delete(requestId);
          resolve(false);
        }
      } catch (e) {
        clearTimeout(timer);
        this.pendingLockRequests.delete(requestId);
        resolve(false);
      }
    });
  }

  private async handleServerMessage(message: any): Promise<void> {
    // Emit to generic listeners (used by EventJournalReader)
    this.emitMessage(message);

    switch (message.type) {
      case 'BATCH': {
        // Unbatch and process each message
        // Format: [4 bytes: count][4 bytes: len1][msg1][4 bytes: len2][msg2]...
        const batchData = message.data as Uint8Array;
        const view = new DataView(batchData.buffer, batchData.byteOffset, batchData.byteLength);
        let offset = 0;

        const count = view.getUint32(offset, true);
        offset += 4;

        for (let i = 0; i < count; i++) {
          const msgLen = view.getUint32(offset, true);
          offset += 4;

          const msgData = batchData.slice(offset, offset + msgLen);
          offset += msgLen;

          const innerMsg = deserialize(msgData);
          await this.handleServerMessage(innerMsg);
        }
        break;
      }

      case 'AUTH_REQUIRED':
        this.sendAuth();
        break;

      case 'AUTH_ACK': {
        logger.info('Authenticated successfully');
        const wasAuthenticated = this.isAuthenticated();

        // Transition to SYNCING state
        this.stateMachine.transition(SyncState.SYNCING);

        // Reset backoff on successful auth
        this.resetBackoff();

        this.syncPendingOperations();

        // Only re-subscribe on first authentication to prevent UI flickering
        if (!wasAuthenticated) {
          this.startHeartbeat();
          this.startMerkleSync();
          for (const query of this.queries.values()) {
            this.sendQuerySubscription(query);
          }
          for (const topic of this.topics.keys()) {
            this.sendTopicSubscription(topic);
          }
        }

        // After initial sync setup, transition to CONNECTED
        // In a real implementation, you might wait for SYNC_COMPLETE message
        this.stateMachine.transition(SyncState.CONNECTED);
        break;
      }

      case 'PONG': {
        this.handlePong(message);
        break;
      }

      case 'AUTH_FAIL':
        logger.error({ error: message.error }, 'Authentication failed');
        this.authToken = null; // Clear invalid token
        // Stay in AUTHENTICATING or go to ERROR depending on severity
        // For now, let the connection close naturally or retry with new token
        break;

      case 'OP_ACK': {
        const { lastId, achievedLevel, results } = message.payload;
        logger.info({ lastId, achievedLevel, hasResults: !!results }, 'Received ACK for ops');

        // Handle per-operation results if available (Write Concern Phase 5.01)
        if (results && Array.isArray(results)) {
          for (const result of results) {
            const op = this.opLog.find(o => o.id === result.opId);
            if (op && !op.synced) {
              op.synced = true;
              logger.debug({ opId: result.opId, achievedLevel: result.achievedLevel, success: result.success }, 'Op ACK with Write Concern');
            }
            // Resolve pending Write Concern promise if exists
            this.resolveWriteConcernPromise(result.opId, result);
          }
        }

        // Backwards compatible: mark all ops up to lastId as synced
        let maxSyncedId = -1;
        let ackedCount = 0;
        this.opLog.forEach(op => {
          if (op.id && op.id <= lastId) {
            if (!op.synced) {
              ackedCount++;
            }
            op.synced = true;
            const idNum = parseInt(op.id, 10);
            if (!isNaN(idNum) && idNum > maxSyncedId) {
              maxSyncedId = idNum;
            }
          }
        });
        if (maxSyncedId !== -1) {
          this.storageAdapter.markOpsSynced(maxSyncedId).catch(err => logger.error({ err }, 'Failed to mark ops synced'));
        }
        // Check low water mark after ACKs reduce pending count
        if (ackedCount > 0) {
          this.checkLowWaterMark();
        }
        break;
      }

      case 'LOCK_GRANTED': {
        const { requestId, fencingToken } = message.payload;
        const req = this.pendingLockRequests.get(requestId);
        if (req) {
          clearTimeout(req.timer);
          this.pendingLockRequests.delete(requestId);
          req.resolve({ fencingToken });
        }
        break;
      }

      case 'LOCK_RELEASED': {
        const { requestId, success } = message.payload;
        const req = this.pendingLockRequests.get(requestId);
        if (req) {
          clearTimeout(req.timer);
          this.pendingLockRequests.delete(requestId);
          req.resolve(success);
        }
        break;
      }

      case 'QUERY_RESP': {
        const { queryId, results } = message.payload;
        const query = this.queries.get(queryId);
        if (query) {
          query.onResult(results, 'server');
        }
        break;
      }

      case 'QUERY_UPDATE': {
        const { queryId, key, value, type } = message.payload;
        const query = this.queries.get(queryId);
        if (query) {
          query.onUpdate(key, type === 'REMOVE' ? null : value);
        }
        break;
      }

      case 'SERVER_EVENT': {
        // Modified to support ORMap
        const { mapName, eventType, key, record, orRecord, orTag } = message.payload;
        await this.applyServerEvent(mapName, eventType, key, record, orRecord, orTag);
        break;
      }

      case 'SERVER_BATCH_EVENT': {
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
            event.orTag
          );
        }
        break;
      }

      case 'TOPIC_MESSAGE': {
        const { topic, data, publisherId, timestamp } = message.payload;
        const handle = this.topics.get(topic);
        if (handle) {
          handle.onMessage(data, { publisherId, timestamp });
        }
        break;
      }

      case 'GC_PRUNE': {
        const { olderThan } = message.payload;
        logger.info({ olderThan: olderThan.millis }, 'Received GC_PRUNE request');

        for (const [name, map] of this.maps) {
          if (map instanceof LWWMap) {
            const removedKeys = map.prune(olderThan);
            for (const key of removedKeys) {
              await this.storageAdapter.remove(`${name}:${key}`);
            }
            if (removedKeys.length > 0) {
              logger.info({ mapName: name, count: removedKeys.length }, 'Pruned tombstones from LWWMap');
            }
          } else if (map instanceof ORMap) {
            const removedTags = map.prune(olderThan);
            if (removedTags.length > 0) {
              logger.info({ mapName: name, count: removedTags.length }, 'Pruned tombstones from ORMap');
            }
          }
        }
        break;
      }

      case 'SYNC_RESET_REQUIRED': {
        const { mapName } = message.payload;
        logger.warn({ mapName }, 'Sync Reset Required due to GC Age');
        await this.resetMap(mapName);
        // Trigger re-sync as fresh
        this.sendMessage({
          type: 'SYNC_INIT',
          mapName,
          lastSyncTimestamp: 0
        });
        break;
      }

      case 'SYNC_RESP_ROOT': {
        const { mapName, rootHash, timestamp } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof LWWMap) {
          const localRootHash = map.getMerkleTree().getRootHash();
          if (localRootHash !== rootHash) {
            logger.info({ mapName, localRootHash, remoteRootHash: rootHash }, 'Root hash mismatch, requesting buckets');
            this.sendMessage({
              type: 'MERKLE_REQ_BUCKET',
              payload: { mapName, path: '' }
            });
          } else {
            logger.info({ mapName }, 'Map is in sync');
          }
        }
        // Update HLC with server timestamp
        if (timestamp) {
          this.hlc.update(timestamp);
          this.lastSyncTimestamp = timestamp.millis;
          await this.saveOpLog();
        }
        break;
      }

      case 'SYNC_RESP_BUCKETS': {
        const { mapName, path, buckets } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof LWWMap) {
          const tree = map.getMerkleTree();
          const localBuckets = tree.getBuckets(path);

          for (const [bucketKey, remoteHash] of Object.entries(buckets)) {
            const localHash = localBuckets[bucketKey] || 0;
            if (localHash !== remoteHash) {
              const newPath = path + bucketKey;
              this.sendMessage({
                type: 'MERKLE_REQ_BUCKET',
                payload: { mapName, path: newPath }
              });
            }
          }
        }
        break;
      }

      case 'SYNC_RESP_LEAF': {
        const { mapName, records } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof LWWMap) {
          let updateCount = 0;
          for (const { key, record } of records) {
            // Merge into local map
            const updated = map.merge(key, record);
            if (updated) {
              updateCount++;
              // Persist to storage
              await this.storageAdapter.put(`${mapName}:${key}`, record);
            }
          }
          if (updateCount > 0) {
            logger.info({ mapName, count: updateCount }, 'Synced records from server');
          }
        }
        break;
      }

      // ============ ORMap Sync Message Handlers ============

      case 'ORMAP_SYNC_RESP_ROOT': {
        const { mapName, rootHash, timestamp } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof ORMap) {
          const localTree = map.getMerkleTree();
          const localRootHash = localTree.getRootHash();

          if (localRootHash !== rootHash) {
            logger.info({ mapName, localRootHash, remoteRootHash: rootHash }, 'ORMap root hash mismatch, requesting buckets');
            this.sendMessage({
              type: 'ORMAP_MERKLE_REQ_BUCKET',
              payload: { mapName, path: '' }
            });
          } else {
            logger.info({ mapName }, 'ORMap is in sync');
          }
        }
        // Update HLC with server timestamp
        if (timestamp) {
          this.hlc.update(timestamp);
          this.lastSyncTimestamp = timestamp.millis;
          await this.saveOpLog();
        }
        break;
      }

      case 'ORMAP_SYNC_RESP_BUCKETS': {
        const { mapName, path, buckets } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof ORMap) {
          const tree = map.getMerkleTree();
          const localBuckets = tree.getBuckets(path);

          for (const [bucketKey, remoteHash] of Object.entries(buckets)) {
            const localHash = localBuckets[bucketKey] || 0;
            if (localHash !== remoteHash) {
              const newPath = path + bucketKey;
              this.sendMessage({
                type: 'ORMAP_MERKLE_REQ_BUCKET',
                payload: { mapName, path: newPath }
              });
            }
          }

          // Also check for buckets that exist locally but not on remote
          for (const [bucketKey, localHash] of Object.entries(localBuckets)) {
            if (!(bucketKey in buckets) && localHash !== 0) {
              // Local has data that remote doesn't - need to push
              const newPath = path + bucketKey;
              const keys = tree.getKeysInBucket(newPath);
              if (keys.length > 0) {
                this.pushORMapDiff(mapName, keys, map);
              }
            }
          }
        }
        break;
      }

      case 'ORMAP_SYNC_RESP_LEAF': {
        const { mapName, entries } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof ORMap) {
          let totalAdded = 0;
          let totalUpdated = 0;

          for (const entry of entries) {
            const { key, records, tombstones } = entry;
            const result = map.mergeKey(key, records, tombstones);
            totalAdded += result.added;
            totalUpdated += result.updated;
          }

          if (totalAdded > 0 || totalUpdated > 0) {
            logger.info({ mapName, added: totalAdded, updated: totalUpdated }, 'Synced ORMap records from server');
          }

          // Now push any local records that server might not have
          const keysToCheck = entries.map((e: { key: string }) => e.key);
          await this.pushORMapDiff(mapName, keysToCheck, map);
        }
        break;
      }

      case 'ORMAP_DIFF_RESPONSE': {
        const { mapName, entries } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof ORMap) {
          let totalAdded = 0;
          let totalUpdated = 0;

          for (const entry of entries) {
            const { key, records, tombstones } = entry;
            const result = map.mergeKey(key, records, tombstones);
            totalAdded += result.added;
            totalUpdated += result.updated;
          }

          if (totalAdded > 0 || totalUpdated > 0) {
            logger.info({ mapName, added: totalAdded, updated: totalUpdated }, 'Merged ORMap diff from server');
          }
        }
        break;
      }

      // ============ PN Counter Message Handlers (Phase 5.2) ============

      case 'COUNTER_UPDATE': {
        const { name, state } = message.payload;
        logger.debug({ name }, 'Received COUNTER_UPDATE');
        this.handleCounterUpdate(name, state);
        break;
      }

      case 'COUNTER_RESPONSE': {
        // Initial counter state response
        const { name, state } = message.payload;
        logger.debug({ name }, 'Received COUNTER_RESPONSE');
        this.handleCounterUpdate(name, state);
        break;
      }

      // ============ Entry Processor Message Handlers (Phase 5.03) ============

      case 'ENTRY_PROCESS_RESPONSE': {
        logger.debug({ requestId: message.requestId, success: message.success }, 'Received ENTRY_PROCESS_RESPONSE');
        this.handleEntryProcessResponse(message);
        break;
      }

      case 'ENTRY_PROCESS_BATCH_RESPONSE': {
        logger.debug({ requestId: message.requestId }, 'Received ENTRY_PROCESS_BATCH_RESPONSE');
        this.handleEntryProcessBatchResponse(message);
        break;
      }

      // ============ Conflict Resolver Message Handlers (Phase 5.05) ============

      case 'REGISTER_RESOLVER_RESPONSE': {
        logger.debug({ requestId: message.requestId, success: message.success }, 'Received REGISTER_RESOLVER_RESPONSE');
        this.conflictResolverClient.handleRegisterResponse(message);
        break;
      }

      case 'UNREGISTER_RESOLVER_RESPONSE': {
        logger.debug({ requestId: message.requestId, success: message.success }, 'Received UNREGISTER_RESOLVER_RESPONSE');
        this.conflictResolverClient.handleUnregisterResponse(message);
        break;
      }

      case 'LIST_RESOLVERS_RESPONSE': {
        logger.debug({ requestId: message.requestId }, 'Received LIST_RESOLVERS_RESPONSE');
        this.conflictResolverClient.handleListResponse(message);
        break;
      }

      case 'MERGE_REJECTED': {
        logger.debug({ mapName: message.mapName, key: message.key, reason: message.reason }, 'Received MERGE_REJECTED');
        this.conflictResolverClient.handleMergeRejected(message);
        break;
      }

      // ============ Full-Text Search Message Handlers (Phase 11.1a) ============

      case 'SEARCH_RESP': {
        logger.debug({ requestId: message.payload?.requestId, resultCount: message.payload?.results?.length }, 'Received SEARCH_RESP');
        this.handleSearchResponse(message.payload);
        break;
      }
    }

    if (message.timestamp) {
      this.hlc.update(message.timestamp);
      this.lastSyncTimestamp = message.timestamp.millis;
      await this.saveOpLog();
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
    eventType: string,
    key: string,
    record?: any,
    orRecord?: any,
    orTag?: string
  ): Promise<void> {
    const localMap = this.maps.get(mapName);
    if (localMap) {
      if (localMap instanceof LWWMap && record) {
        localMap.merge(key, record);
        await this.storageAdapter.put(`${mapName}:${key}`, record);
      } else if (localMap instanceof ORMap) {
        if (eventType === 'OR_ADD' && orRecord) {
          localMap.apply(key, orRecord);
          // We need to store ORMap records differently in storageAdapter or use a convention
          // For now, skipping persistent storage update for ORMap in this example
        } else if (eventType === 'OR_REMOVE' && orTag) {
          localMap.applyTombstone(orTag);
        }
      }
    }
  }

  /**
   * Closes the WebSocket connection and cleans up resources.
   */
  public close(): void {
    this.stopHeartbeat();

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.useConnectionProvider) {
      // Close via connection provider
      this.connectionProvider.close().catch((err) => {
        logger.error({ err }, 'Error closing ConnectionProvider');
      });
    } else if (this.websocket) {
      // Legacy: close direct websocket
      this.websocket.onclose = null; // Prevent reconnect on intentional close
      this.websocket.close();
      this.websocket = null;
    }

    // Cancel pending Write Concern promises (Phase 5.01)
    this.cancelAllWriteConcernPromises(new Error('SyncEngine closed'));

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
    this.resetBackoff();
    if (this.useConnectionProvider) {
      this.initConnectionProvider();
    } else {
      this.initConnection();
    }
  }

  // ============================================
  // Failover Support Methods (Phase 4.5 Task 05)
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

      const handler = () => {
        clearTimeout(timeout);
        this.connectionProvider.off('partitionMapUpdated', handler);
        resolve();
      };

      this.connectionProvider.on('partitionMapUpdated', handler);
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
      // If already connected, resolve immediately
      if (this.connectionProvider.isConnected()) {
        resolve();
        return;
      }

      const timeout = setTimeout(() => {
        this.connectionProvider.off('connected', handler);
        reject(new Error('Connection timeout waiting for reconnection'));
      }, timeoutMs);

      const handler = () => {
        clearTimeout(timeout);
        this.connectionProvider.off('connected', handler);
        resolve();
      };

      this.connectionProvider.on('connected', handler);
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
    return this.connectionProvider.isConnected();
  }

  /**
   * Get the connection provider for direct access.
   * Use with caution - prefer using SyncEngine methods.
   */
  public getConnectionProvider(): IConnectionProvider {
    return this.connectionProvider;
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
    const mapKeys = allKeys.filter(k => k.startsWith(mapName + ':'));
    for (const key of mapKeys) {
      await this.storageAdapter.remove(key);
    }
    logger.info({ mapName, removedStorageCount: mapKeys.length }, 'Reset map: Cleared memory and storage');
  }

  // ============ Heartbeat Methods ============

  /**
   * Starts the heartbeat mechanism after successful connection.
   */
  private startHeartbeat(): void {
    if (!this.heartbeatConfig.enabled) {
      return;
    }

    this.stopHeartbeat(); // Clear any existing interval
    this.lastPongReceived = Date.now();

    this.heartbeatInterval = setInterval(() => {
      this.sendPing();
      this.checkHeartbeatTimeout();
    }, this.heartbeatConfig.intervalMs);

    logger.info({ intervalMs: this.heartbeatConfig.intervalMs }, 'Heartbeat started');
  }

  /**
   * Stops the heartbeat mechanism.
   */
  private stopHeartbeat(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
      logger.info('Heartbeat stopped');
    }
  }

  /**
   * Sends a PING message to the server.
   */
  private sendPing(): void {
    if (this.canSend()) {
      const pingMessage = {
        type: 'PING',
        timestamp: Date.now(),
      };
      this.sendMessage(pingMessage);
    }
  }

  /**
   * Handles incoming PONG message from server.
   */
  private handlePong(msg: { timestamp: number; serverTime: number }): void {
    const now = Date.now();
    this.lastPongReceived = now;
    this.lastRoundTripTime = now - msg.timestamp;

    logger.debug({
      rtt: this.lastRoundTripTime,
      serverTime: msg.serverTime,
      clockSkew: msg.serverTime - (msg.timestamp + this.lastRoundTripTime / 2),
    }, 'Received PONG');
  }

  /**
   * Checks if heartbeat has timed out and triggers reconnection if needed.
   */
  private checkHeartbeatTimeout(): void {
    const now = Date.now();
    const timeSinceLastPong = now - this.lastPongReceived;

    if (timeSinceLastPong > this.heartbeatConfig.timeoutMs) {
      logger.warn({
        timeSinceLastPong,
        timeoutMs: this.heartbeatConfig.timeoutMs,
      }, 'Heartbeat timeout - triggering reconnection');

      this.stopHeartbeat();

      // Force close and reconnect
      if (this.websocket) {
        this.websocket.close();
      }
    }
  }

  /**
   * Returns the last measured round-trip time in milliseconds.
   * Returns null if no PONG has been received yet.
   */
  public getLastRoundTripTime(): number | null {
    return this.lastRoundTripTime;
  }

  /**
   * Returns true if the connection is considered healthy based on heartbeat.
   * A connection is healthy if it's online, authenticated, and has received
   * a PONG within the timeout window.
   */
  public isConnectionHealthy(): boolean {
    if (!this.isOnline() || !this.isAuthenticated()) {
      return false;
    }

    if (!this.heartbeatConfig.enabled) {
      return true; // If heartbeat disabled, consider healthy if online
    }

    const timeSinceLastPong = Date.now() - this.lastPongReceived;
    return timeSinceLastPong < this.heartbeatConfig.timeoutMs;
  }

  // ============ ORMap Sync Methods ============

  /**
   * Push local ORMap diff to server for the given keys.
   * Sends local records and tombstones that the server might not have.
   */
  private async pushORMapDiff(
    mapName: string,
    keys: string[],
    map: ORMap<any, any>
  ): Promise<void> {
    const entries: Array<{
      key: string;
      records: ORMapRecord<any>[];
      tombstones: string[];
    }> = [];

    const snapshot = map.getSnapshot();

    for (const key of keys) {
      const recordsMap = map.getRecordsMap(key);
      if (recordsMap && recordsMap.size > 0) {
        // Get records as array
        const records = Array.from(recordsMap.values());

        // Get tombstones relevant to this key's records
        // (tombstones that match tags that were in this key)
        const tombstones: string[] = [];
        for (const tag of snapshot.tombstones) {
          // Include all tombstones - server will filter
          tombstones.push(tag);
        }

        entries.push({
          key,
          records,
          tombstones
        });
      }
    }

    if (entries.length > 0) {
      this.sendMessage({
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries
        }
      });
      logger.debug({ mapName, keyCount: entries.length }, 'Pushed ORMap diff to server');
    }
  }

  // ============ Backpressure Methods ============

  /**
   * Get the current number of pending (unsynced) operations.
   */
  public getPendingOpsCount(): number {
    return this.opLog.filter(op => !op.synced).length;
  }

  /**
   * Get the current backpressure status.
   */
  public getBackpressureStatus(): BackpressureStatus {
    const pending = this.getPendingOpsCount();
    const max = this.backpressureConfig.maxPendingOps;
    return {
      pending,
      max,
      percentage: max > 0 ? pending / max : 0,
      isPaused: this.backpressurePaused,
      strategy: this.backpressureConfig.strategy,
    };
  }

  /**
   * Returns true if writes are currently paused due to backpressure.
   */
  public isBackpressurePaused(): boolean {
    return this.backpressurePaused;
  }

  /**
   * Subscribe to backpressure events.
   * @param event Event name: 'backpressure:high', 'backpressure:low', 'backpressure:paused', 'backpressure:resumed', 'operation:dropped'
   * @param listener Callback function
   * @returns Unsubscribe function
   */
  public onBackpressure(
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    listener: (data?: BackpressureThresholdEvent | OperationDroppedEvent) => void
  ): () => void {
    if (!this.backpressureListeners.has(event)) {
      this.backpressureListeners.set(event, new Set());
    }
    this.backpressureListeners.get(event)!.add(listener);

    return () => {
      this.backpressureListeners.get(event)?.delete(listener);
    };
  }

  /**
   * Emit a backpressure event to all listeners.
   */
  private emitBackpressureEvent(
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    data?: BackpressureThresholdEvent | OperationDroppedEvent
  ): void {
    const listeners = this.backpressureListeners.get(event);
    if (listeners) {
      for (const listener of listeners) {
        try {
          listener(data);
        } catch (err) {
          logger.error({ err, event }, 'Error in backpressure event listener');
        }
      }
    }
  }

  /**
   * Check backpressure before adding a new operation.
   * May pause, throw, or drop depending on strategy.
   */
  private async checkBackpressure(): Promise<void> {
    const pendingCount = this.getPendingOpsCount();

    if (pendingCount < this.backpressureConfig.maxPendingOps) {
      return; // Capacity available
    }

    switch (this.backpressureConfig.strategy) {
      case 'pause':
        await this.waitForCapacity();
        break;
      case 'throw':
        throw new BackpressureError(
          pendingCount,
          this.backpressureConfig.maxPendingOps
        );
      case 'drop-oldest':
        this.dropOldestOp();
        break;
    }
  }

  /**
   * Check high water mark and emit event if threshold reached.
   */
  private checkHighWaterMark(): void {
    const pendingCount = this.getPendingOpsCount();
    const threshold = Math.floor(
      this.backpressureConfig.maxPendingOps * this.backpressureConfig.highWaterMark
    );

    if (pendingCount >= threshold && !this.highWaterMarkEmitted) {
      this.highWaterMarkEmitted = true;
      logger.warn(
        { pending: pendingCount, max: this.backpressureConfig.maxPendingOps },
        'Backpressure high water mark reached'
      );
      this.emitBackpressureEvent('backpressure:high', {
        pending: pendingCount,
        max: this.backpressureConfig.maxPendingOps,
      });
    }
  }

  /**
   * Check low water mark and resume paused writes if threshold reached.
   */
  private checkLowWaterMark(): void {
    const pendingCount = this.getPendingOpsCount();
    const lowThreshold = Math.floor(
      this.backpressureConfig.maxPendingOps * this.backpressureConfig.lowWaterMark
    );
    const highThreshold = Math.floor(
      this.backpressureConfig.maxPendingOps * this.backpressureConfig.highWaterMark
    );

    // Reset high water mark flag when below high threshold
    if (pendingCount < highThreshold && this.highWaterMarkEmitted) {
      this.highWaterMarkEmitted = false;
    }

    // Emit low water mark event when crossing below threshold
    if (pendingCount <= lowThreshold) {
      if (this.backpressurePaused) {
        this.backpressurePaused = false;
        logger.info(
          { pending: pendingCount, max: this.backpressureConfig.maxPendingOps },
          'Backpressure low water mark reached, resuming writes'
        );
        this.emitBackpressureEvent('backpressure:low', {
          pending: pendingCount,
          max: this.backpressureConfig.maxPendingOps,
        });
        this.emitBackpressureEvent('backpressure:resumed');

        // Resume all waiting writes
        const waiting = this.waitingForCapacity;
        this.waitingForCapacity = [];
        for (const resolve of waiting) {
          resolve();
        }
      }
    }
  }

  /**
   * Wait for capacity to become available (used by 'pause' strategy).
   */
  private async waitForCapacity(): Promise<void> {
    if (!this.backpressurePaused) {
      this.backpressurePaused = true;
      logger.warn('Backpressure paused - waiting for capacity');
      this.emitBackpressureEvent('backpressure:paused');
    }

    return new Promise<void>((resolve) => {
      this.waitingForCapacity.push(resolve);
    });
  }

  /**
   * Drop the oldest pending operation (used by 'drop-oldest' strategy).
   */
  private dropOldestOp(): void {
    // Find oldest unsynced operation by array order (oldest first)
    const oldestIndex = this.opLog.findIndex(op => !op.synced);

    if (oldestIndex !== -1) {
      const dropped = this.opLog[oldestIndex];
      this.opLog.splice(oldestIndex, 1);

      logger.warn(
        { opId: dropped.id, mapName: dropped.mapName, key: dropped.key },
        'Dropped oldest pending operation due to backpressure'
      );

      this.emitBackpressureEvent('operation:dropped', {
        opId: dropped.id,
        mapName: dropped.mapName,
        opType: dropped.opType,
        key: dropped.key,
      });
    }
  }

  // ============================================
  // Write Concern Methods (Phase 5.01)
  // ============================================

  /**
   * Register a pending Write Concern promise for an operation.
   * The promise will be resolved when the server sends an ACK with the operation result.
   *
   * @param opId - Operation ID
   * @param timeout - Timeout in ms (default: 5000)
   * @returns Promise that resolves with the Write Concern result
   */
  public registerWriteConcernPromise(opId: string, timeout: number = 5000): Promise<any> {
    return new Promise((resolve, reject) => {
      const timeoutHandle = setTimeout(() => {
        this.pendingWriteConcernPromises.delete(opId);
        reject(new Error(`Write Concern timeout for operation ${opId}`));
      }, timeout);

      this.pendingWriteConcernPromises.set(opId, {
        resolve,
        reject,
        timeoutHandle,
      });
    });
  }

  /**
   * Resolve a pending Write Concern promise with the server result.
   *
   * @param opId - Operation ID
   * @param result - Result from server ACK
   */
  private resolveWriteConcernPromise(opId: string, result: any): void {
    const pending = this.pendingWriteConcernPromises.get(opId);
    if (pending) {
      if (pending.timeoutHandle) {
        clearTimeout(pending.timeoutHandle);
      }
      pending.resolve(result);
      this.pendingWriteConcernPromises.delete(opId);
    }
  }

  /**
   * Cancel all pending Write Concern promises (e.g., on disconnect).
   */
  private cancelAllWriteConcernPromises(error: Error): void {
    for (const [opId, pending] of this.pendingWriteConcernPromises.entries()) {
      if (pending.timeoutHandle) {
        clearTimeout(pending.timeoutHandle);
      }
      pending.reject(error);
    }
    this.pendingWriteConcernPromises.clear();
  }

  // ============================================
  // PN Counter Methods (Phase 5.2)
  // ============================================

  /** Counter update listeners by name */
  private counterUpdateListeners: Map<string, Set<(state: any) => void>> = new Map();

  /**
   * Subscribe to counter updates from server.
   * @param name Counter name
   * @param listener Callback when counter state is updated
   * @returns Unsubscribe function
   */
  public onCounterUpdate(name: string, listener: (state: any) => void): () => void {
    if (!this.counterUpdateListeners.has(name)) {
      this.counterUpdateListeners.set(name, new Set());
    }
    this.counterUpdateListeners.get(name)!.add(listener);

    return () => {
      this.counterUpdateListeners.get(name)?.delete(listener);
      if (this.counterUpdateListeners.get(name)?.size === 0) {
        this.counterUpdateListeners.delete(name);
      }
    };
  }

  /**
   * Request initial counter state from server.
   * @param name Counter name
   */
  public requestCounter(name: string): void {
    if (this.isAuthenticated()) {
      this.sendMessage({
        type: 'COUNTER_REQUEST',
        payload: { name }
      });
    }
  }

  /**
   * Sync local counter state to server.
   * @param name Counter name
   * @param state Counter state to sync
   */
  public syncCounter(name: string, state: any): void {
    if (this.isAuthenticated()) {
      // Convert Maps to objects for serialization
      const stateObj = {
        positive: Object.fromEntries(state.positive),
        negative: Object.fromEntries(state.negative),
      };

      this.sendMessage({
        type: 'COUNTER_SYNC',
        payload: {
          name,
          state: stateObj
        }
      });
    }
  }

  /**
   * Handle incoming counter update from server.
   * Called by handleServerMessage for COUNTER_UPDATE messages.
   */
  private handleCounterUpdate(name: string, stateObj: { positive: Record<string, number>; negative: Record<string, number> }): void {
    // Convert objects to Maps
    const state = {
      positive: new Map(Object.entries(stateObj.positive)),
      negative: new Map(Object.entries(stateObj.negative)),
    };

    const listeners = this.counterUpdateListeners.get(name);
    if (listeners) {
      for (const listener of listeners) {
        try {
          listener(state);
        } catch (e) {
          logger.error({ err: e, counterName: name }, 'Counter update listener error');
        }
      }
    }
  }

  // ============================================
  // Entry Processor Methods (Phase 5.03)
  // ============================================

  /** Pending entry processor requests by requestId */
  private pendingProcessorRequests: Map<string, {
    resolve: (result: EntryProcessorResult<any>) => void;
    reject: (error: Error) => void;
    timeout: ReturnType<typeof setTimeout>;
  }> = new Map();

  /** Pending batch entry processor requests by requestId */
  private pendingBatchProcessorRequests: Map<string, {
    resolve: (results: Map<string, EntryProcessorResult<any>>) => void;
    reject: (error: Error) => void;
    timeout: ReturnType<typeof setTimeout>;
  }> = new Map();

  /** Default timeout for entry processor requests (ms) */
  private static readonly PROCESSOR_TIMEOUT = 30000;

  /**
   * Execute an entry processor on a single key atomically.
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
    if (!this.isAuthenticated()) {
      return {
        success: false,
        error: 'Not connected to server',
      };
    }

    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      // Set timeout
      const timeout = setTimeout(() => {
        this.pendingProcessorRequests.delete(requestId);
        reject(new Error('Entry processor request timed out'));
      }, SyncEngine.PROCESSOR_TIMEOUT);

      // Store pending request
      this.pendingProcessorRequests.set(requestId, {
        resolve: (result) => {
          clearTimeout(timeout);
          resolve(result);
        },
        reject,
        timeout,
      });

      // Send request
      const sent = this.sendMessage({
        type: 'ENTRY_PROCESS',
        requestId,
        mapName,
        key,
        processor: {
          name: processor.name,
          code: processor.code,
          args: processor.args,
        },
      }, key);

      if (!sent) {
        this.pendingProcessorRequests.delete(requestId);
        clearTimeout(timeout);
        reject(new Error('Failed to send entry processor request'));
      }
    });
  }

  /**
   * Execute an entry processor on multiple keys.
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
    if (!this.isAuthenticated()) {
      const results = new Map<string, EntryProcessorResult<R>>();
      const error: EntryProcessorResult<R> = {
        success: false,
        error: 'Not connected to server',
      };
      for (const key of keys) {
        results.set(key, error);
      }
      return results;
    }

    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      // Set timeout
      const timeout = setTimeout(() => {
        this.pendingBatchProcessorRequests.delete(requestId);
        reject(new Error('Entry processor batch request timed out'));
      }, SyncEngine.PROCESSOR_TIMEOUT);

      // Store pending request
      this.pendingBatchProcessorRequests.set(requestId, {
        resolve: (results) => {
          clearTimeout(timeout);
          resolve(results);
        },
        reject,
        timeout,
      });

      // Send request
      const sent = this.sendMessage({
        type: 'ENTRY_PROCESS_BATCH',
        requestId,
        mapName,
        keys,
        processor: {
          name: processor.name,
          code: processor.code,
          args: processor.args,
        },
      });

      if (!sent) {
        this.pendingBatchProcessorRequests.delete(requestId);
        clearTimeout(timeout);
        reject(new Error('Failed to send entry processor batch request'));
      }
    });
  }

  /**
   * Handle entry processor response from server.
   * Called by handleServerMessage for ENTRY_PROCESS_RESPONSE messages.
   */
  private handleEntryProcessResponse(message: {
    requestId: string;
    success: boolean;
    result?: unknown;
    newValue?: unknown;
    error?: string;
  }): void {
    const pending = this.pendingProcessorRequests.get(message.requestId);
    if (pending) {
      this.pendingProcessorRequests.delete(message.requestId);
      pending.resolve({
        success: message.success,
        result: message.result,
        newValue: message.newValue,
        error: message.error,
      });
    }
  }

  /**
   * Handle entry processor batch response from server.
   * Called by handleServerMessage for ENTRY_PROCESS_BATCH_RESPONSE messages.
   */
  private handleEntryProcessBatchResponse(message: {
    requestId: string;
    results: Record<string, EntryProcessKeyResult>;
  }): void {
    const pending = this.pendingBatchProcessorRequests.get(message.requestId);
    if (pending) {
      this.pendingBatchProcessorRequests.delete(message.requestId);

      // Convert Record to Map
      const resultsMap = new Map<string, EntryProcessorResult<any>>();
      for (const [key, result] of Object.entries(message.results)) {
        resultsMap.set(key, {
          success: result.success,
          result: result.result,
          newValue: result.newValue,
          error: result.error,
        });
      }

      pending.resolve(resultsMap);
    }
  }

  // ============================================
  // Event Journal Methods (Phase 5.04)
  // ============================================

  /** Message listeners for journal and other generic messages */
  private messageListeners: Set<(message: any) => void> = new Set();

  /**
   * Subscribe to all incoming messages.
   * Used by EventJournalReader to receive journal events.
   *
   * @param event Event type (currently only 'message')
   * @param handler Message handler
   */
  public on(event: 'message', handler: (message: any) => void): void {
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
  public off(event: 'message', handler: (message: any) => void): void {
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
  public send(message: any): void {
    this.sendMessage(message);
  }

  /**
   * Emit message to all listeners.
   * Called internally when a message is received.
   */
  private emitMessage(message: any): void {
    for (const listener of this.messageListeners) {
      try {
        listener(message);
      } catch (e) {
        logger.error({ err: e }, 'Message listener error');
      }
    }
  }

  // ============================================
  // Full-Text Search Methods (Phase 11.1a)
  // ============================================

  /** Pending search requests by requestId */
  private pendingSearchRequests: Map<string, {
    resolve: (result: SearchResult<unknown>[]) => void;
    reject: (error: Error) => void;
    timeout: ReturnType<typeof setTimeout>;
  }> = new Map();

  /** Default timeout for search requests (ms) */
  private static readonly SEARCH_TIMEOUT = 30000;

  /**
   * Perform a one-shot BM25 search on the server.
   *
   * @param mapName Name of the map to search
   * @param query Search query text
   * @param options Search options (limit, minScore, boost)
   * @returns Promise resolving to search results
   */
  public async search<T>(
    mapName: string,
    query: string,
    options?: SearchOptions
  ): Promise<SearchResult<T>[]> {
    if (!this.isAuthenticated()) {
      throw new Error('Not connected to server');
    }

    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      // Set timeout
      const timeout = setTimeout(() => {
        this.pendingSearchRequests.delete(requestId);
        reject(new Error('Search request timed out'));
      }, SyncEngine.SEARCH_TIMEOUT);

      // Store pending request
      this.pendingSearchRequests.set(requestId, {
        resolve: (results) => {
          clearTimeout(timeout);
          resolve(results as SearchResult<T>[]);
        },
        reject: (error) => {
          clearTimeout(timeout);
          reject(error);
        },
        timeout,
      });

      // Send search request
      const sent = this.sendMessage({
        type: 'SEARCH',
        payload: {
          requestId,
          mapName,
          query,
          options,
        },
      });

      if (!sent) {
        this.pendingSearchRequests.delete(requestId);
        clearTimeout(timeout);
        reject(new Error('Failed to send search request'));
      }
    });
  }

  /**
   * Handle search response from server.
   */
  private handleSearchResponse(payload: {
    requestId: string;
    results: SearchResult<unknown>[];
    totalCount: number;
    error?: string;
  }): void {
    const pending = this.pendingSearchRequests.get(payload.requestId);
    if (pending) {
      this.pendingSearchRequests.delete(payload.requestId);

      if (payload.error) {
        pending.reject(new Error(payload.error));
      } else {
        pending.resolve(payload.results);
      }
    }
  }

  // ============================================
  // Conflict Resolver Client (Phase 5.05)
  // ============================================

  /**
   * Get the conflict resolver client for registering custom resolvers
   * and subscribing to merge rejection events.
   */
  public getConflictResolverClient(): ConflictResolverClient {
    return this.conflictResolverClient;
  }
}
