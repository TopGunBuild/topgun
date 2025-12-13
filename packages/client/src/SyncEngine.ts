import { HLC, LWWMap, ORMap, serialize, deserialize, evaluatePredicate } from '@topgunbuild/core';
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
  serverUrl: string;
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

  constructor(config: SyncEngineConfig) {
    this.nodeId = config.nodeId;
    this.serverUrl = config.serverUrl;
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

    this.initConnection();
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

    if (this.websocket?.readyState === WebSocket.OPEN) {
      this.websocket.send(serialize({
        type: 'OP_BATCH',
        payload: {
          ops: pending
        }
      }));
    }
  }

  private startMerkleSync(): void {
    for (const [mapName, map] of this.maps) {
      if (map instanceof LWWMap) {
        logger.info({ mapName }, 'Starting Merkle sync for map');
        this.websocket?.send(serialize({
          type: 'SYNC_INIT',
          mapName,
          lastSyncTimestamp: this.lastSyncTimestamp
        }));
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

    this.websocket?.send(serialize({
      type: 'AUTH',
      token
    }));
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
      this.websocket?.send(serialize({
        type: 'TOPIC_UNSUB',
        payload: { topic }
      }));
    }
  }

  public publishTopic(topic: string, data: any) {
    if (this.isAuthenticated()) {
      this.websocket?.send(serialize({
        type: 'TOPIC_PUB',
        payload: { topic, data }
      }));
    } else {
      // TODO: Queue topic messages or drop?
      // Spec says Fire-and-Forget, so dropping is acceptable if offline,
      // but queueing is better UX.
      // For now, log warning.
      logger.warn({ topic }, 'Dropped topic publish (offline)');
    }
  }

  private sendTopicSubscription(topic: string) {
    this.websocket?.send(serialize({
      type: 'TOPIC_SUB',
      payload: { topic }
    }));
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
      this.websocket?.send(serialize({
        type: 'QUERY_UNSUB',
        payload: { queryId }
      }));
    }
  }

  private sendQuerySubscription(query: QueryHandle<any>) {
    this.websocket?.send(serialize({
      type: 'QUERY_SUB',
      payload: {
        queryId: query.id,
        mapName: query.getMapName(),
        query: query.getFilter()
      }
    }));
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
        this.websocket?.send(serialize({
          type: 'LOCK_REQUEST',
          payload: { requestId, name, ttl }
        }));
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
        this.websocket?.send(serialize({
          type: 'LOCK_RELEASE',
          payload: { requestId, name, fencingToken }
        }));
      } catch (e) {
        clearTimeout(timer);
        this.pendingLockRequests.delete(requestId);
        resolve(false);
      }
    });
  }

  private async handleServerMessage(message: any): Promise<void> {
    switch (message.type) {
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
        const { lastId } = message.payload;
        logger.info({ lastId }, 'Received ACK for ops');
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
        this.websocket?.send(serialize({
          type: 'SYNC_INIT',
          mapName,
          lastSyncTimestamp: 0
        }));
        break;
      }

      case 'SYNC_RESP_ROOT': {
        const { mapName, rootHash, timestamp } = message.payload;
        const map = this.maps.get(mapName);
        if (map instanceof LWWMap) {
          const localRootHash = map.getMerkleTree().getRootHash();
          if (localRootHash !== rootHash) {
            logger.info({ mapName, localRootHash, remoteRootHash: rootHash }, 'Root hash mismatch, requesting buckets');
            this.websocket?.send(serialize({
              type: 'MERKLE_REQ_BUCKET',
              payload: { mapName, path: '' }
            }));
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
              this.websocket?.send(serialize({
                type: 'MERKLE_REQ_BUCKET',
                payload: { mapName, path: newPath }
              }));
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
   * Closes the WebSocket connection and cleans up resources.
   */
  public close(): void {
    this.stopHeartbeat();

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.websocket) {
      this.websocket.onclose = null; // Prevent reconnect on intentional close
      this.websocket.close();
      this.websocket = null;
    }

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
    this.initConnection();
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
    if (this.websocket?.readyState === WebSocket.OPEN) {
      const pingMessage = {
        type: 'PING',
        timestamp: Date.now(),
      };
      this.websocket.send(serialize(pingMessage));
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
}
