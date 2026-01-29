import { HLC, LWWMap, ORMap, deserialize, evaluatePredicate } from '@topgunbuild/core';
import type { EntryProcessorDef, EntryProcessorResult, SearchOptions } from '@topgunbuild/core';
import type { LWWRecord, ORMapRecord, Timestamp } from '@topgunbuild/core';
import type { IStorageAdapter } from './IStorageAdapter';
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
import { BackpressureError } from './errors/BackpressureError';
import type { IConnectionProvider } from './types';
import { ConflictResolverClient } from './ConflictResolverClient';
import { WebSocketManager, BackpressureController, QueryManager, TopicManager, LockManager, WriteConcernManager, CounterManager, EntryProcessorClient, SearchClient } from './sync';
import type { SearchResult } from './sync';

// Re-export SearchResult from sync module for backwards compatibility
export type { SearchResult } from './sync';

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
  /** @deprecated Use connectionProvider instead */
  serverUrl?: string;
  /** Connection provider (preferred over serverUrl) */
  connectionProvider?: IConnectionProvider;
  storageAdapter: IStorageAdapter;
  reconnectInterval?: number;
  heartbeat?: Partial<HeartbeatConfig>;
  backoff?: Partial<BackoffConfig>;
  backpressure?: Partial<BackpressureConfig>;
  /** Configuration for offline topic message queue */
  topicQueue?: Partial<TopicQueueConfig>;
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
  private readonly storageAdapter: IStorageAdapter;
  private readonly hlc: HLC;
  private readonly stateMachine: SyncStateMachine;
  private readonly heartbeatConfig: HeartbeatConfig;
  private readonly backoffConfig: BackoffConfig;

  // WebSocketManager handles all connection/websocket operations
  private readonly webSocketManager: WebSocketManager;

  // QueryManager handles all query operations (Phase 05-02)
  private readonly queryManager: QueryManager;

  // TopicManager handles all topic (pub/sub) operations (Phase 09a)
  private readonly topicManager: TopicManager;

  // LockManager handles distributed lock operations (Phase 09a)
  private readonly lockManager: LockManager;

  // WriteConcernManager handles write concern tracking (Phase 09a)
  private readonly writeConcernManager: WriteConcernManager;

  // CounterManager handles PN counter operations (Phase 09b)
  private readonly counterManager: CounterManager;

  // EntryProcessorClient handles entry processor operations (Phase 09b)
  private readonly entryProcessorClient: EntryProcessorClient;

  // SearchClient handles full-text search operations (Phase 09b)
  private readonly searchClient: SearchClient;

  private opLog: OpLogEntry[] = [];
  private maps: Map<string, LWWMap<any, any> | ORMap<any, any>> = new Map();
  private lastSyncTimestamp: number = 0;
  private authToken: string | null = null;
  private tokenProvider: (() => Promise<string | null>) | null = null;

  // BackpressureController handles all backpressure operations (Phase 05-03)
  private readonly backpressureConfig: BackpressureConfig;
  private readonly backpressureController: BackpressureController;

  // Conflict Resolver client (Phase 5.05)
  private readonly conflictResolverClient: ConflictResolverClient;

  constructor(config: SyncEngineConfig) {
    // Validate config: either serverUrl or connectionProvider required
    if (!config.serverUrl && !config.connectionProvider) {
      throw new Error('SyncEngine requires either serverUrl or connectionProvider');
    }

    this.nodeId = config.nodeId;
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

    // Initialize BackpressureController with shared opLog reference (Phase 05-03)
    this.backpressureController = new BackpressureController({
      config: this.backpressureConfig,
      opLog: this.opLog, // Pass reference, not copy
    });

    // Merge topic queue config with defaults (Phase 3 BUG-06)
    const topicQueueConfig: TopicQueueConfig = {
      ...DEFAULT_TOPIC_QUEUE_CONFIG,
      ...config.topicQueue,
    };

    // Initialize WebSocketManager with callbacks to SyncEngine
    this.webSocketManager = new WebSocketManager({
      serverUrl: config.serverUrl,
      connectionProvider: config.connectionProvider,
      stateMachine: this.stateMachine,
      backoffConfig: this.backoffConfig,
      heartbeatConfig: this.heartbeatConfig,
      onMessage: (msg) => this.handleServerMessage(msg),
      onConnected: () => this.handleConnectionEstablished(),
      onDisconnected: () => this.handleConnectionLost(),
      onReconnected: () => this.handleReconnection(),
    });

    // Initialize QueryManager with callbacks (Phase 05-02)
    this.queryManager = new QueryManager({
      storageAdapter: this.storageAdapter,
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize TopicManager with callbacks (Phase 09a)
    this.topicManager = new TopicManager({
      topicQueueConfig,
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize LockManager with callbacks (Phase 09a)
    this.lockManager = new LockManager({
      sendMessage: (msg, key) => this.webSocketManager.sendMessage(msg, key),
      isAuthenticated: () => this.isAuthenticated(),
      isOnline: () => this.isOnline(),
    });

    // Initialize WriteConcernManager (Phase 09a)
    this.writeConcernManager = new WriteConcernManager({
      defaultTimeout: 5000,
    });

    // Initialize CounterManager (Phase 09b)
    this.counterManager = new CounterManager({
      sendMessage: (msg) => this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize EntryProcessorClient (Phase 09b)
    this.entryProcessorClient = new EntryProcessorClient({
      sendMessage: (msg, key) => key !== undefined ? this.sendMessage(msg, key) : this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize SearchClient (Phase 09b)
    this.searchClient = new SearchClient({
      sendMessage: (msg) => this.sendMessage(msg),
      isAuthenticated: () => this.isAuthenticated(),
    });

    // Initialize Conflict Resolver client (Phase 5.05)
    this.conflictResolverClient = new ConflictResolverClient(this);

    // Start connection
    this.webSocketManager.connect();

    this.loadOpLog();
  }

  // ============================================
  // Connection Callbacks (from WebSocketManager)
  // ============================================

  /**
   * Called when connection is established (initial or reconnect).
   */
  private handleConnectionEstablished(): void {
    if (this.authToken || this.tokenProvider) {
      logger.info('Connection established. Sending auth...');
      this.stateMachine.transition(SyncState.AUTHENTICATING);
      this.sendAuth();
    } else {
      logger.info('Connection established. Waiting for auth token...');
      this.stateMachine.transition(SyncState.AUTHENTICATING);
    }
  }

  /**
   * Called when connection is lost.
   */
  private handleConnectionLost(): void {
    // WebSocketManager already stopped heartbeat and transitioned state
    // SyncEngine can do additional cleanup if needed
  }

  /**
   * Called when reconnection succeeds.
   */
  private handleReconnection(): void {
    if (this.authToken || this.tokenProvider) {
      this.stateMachine.transition(SyncState.AUTHENTICATING);
      this.sendAuth();
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
  private sendMessage(message: any, key?: string): boolean {
    return this.webSocketManager.sendMessage(message, key);
  }

  // ============================================
  // Op Log Management
  // ============================================

  private async loadOpLog(): Promise<void> {
    const storedTimestamp = await this.storageAdapter.getMeta('lastSyncTimestamp');
    if (storedTimestamp) {
      this.lastSyncTimestamp = storedTimestamp;
    }

    const pendingOps = await this.storageAdapter.getPendingOps();
    // Clear and push to existing array (preserves BackpressureController reference)
    this.opLog.length = 0;
    for (const op of pendingOps) {
      this.opLog.push({
        ...op,
        id: String(op.id),
        synced: false,
      } as unknown as OpLogEntry);
    }

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

    const id = await this.storageAdapter.appendOpLog(opLogEntry as any);
    opLogEntry.id = String(id);

    this.opLog.push(opLogEntry as OpLogEntry);

    // Check high water mark after adding operation (delegates to BackpressureController)
    this.backpressureController.checkHighWaterMark();

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
      // Force immediate reconnect if we were waiting for retry timer
      logger.info('Auth token set during backoff/disconnect. Reconnecting immediately.');
      this.webSocketManager.clearReconnectTimer();
      // Reset backoff since user provided new credentials
      this.webSocketManager.resetBackoff();
      this.webSocketManager.connect();
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

  /**
   * Subscribe to a standard query.
   * Delegates to QueryManager.
   */
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
  public publishTopic(topic: string, data: any): void {
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
  public async runLocalQuery(mapName: string, filter: QueryFilter): Promise<{ key: string; value: any }[]> {
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
  public requestLock(name: string, requestId: string, ttl: number): Promise<{ fencingToken: number }> {
    return this.lockManager.requestLock(name, requestId, ttl);
  }

  /**
   * Release a distributed lock.
   * Delegates to LockManager.
   */
  public releaseLock(name: string, requestId: string, fencingToken: number): Promise<boolean> {
    return this.lockManager.releaseLock(name, requestId, fencingToken);
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
        this.webSocketManager.resetBackoff();

        this.syncPendingOperations();

        // Flush any queued topic messages from offline period (delegates to TopicManager)
        this.topicManager.flushTopicQueue();

        // Only re-subscribe on first authentication to prevent UI flickering
        if (!wasAuthenticated) {
          this.webSocketManager.startHeartbeat();
          this.startMerkleSync();
          // Re-subscribe all queries via QueryManager
          this.queryManager.resubscribeAll();
          // Re-subscribe topics via TopicManager
          this.topicManager.resubscribeAll();
        }

        // After initial sync setup, transition to CONNECTED
        // In a real implementation, you might wait for SYNC_COMPLETE message
        this.stateMachine.transition(SyncState.CONNECTED);
        break;
      }

      case 'PONG': {
        // PONG is handled internally by WebSocketManager for RTT tracking
        // Message still comes here for any additional processing if needed
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
            // Resolve pending Write Concern promise if exists (delegates to WriteConcernManager)
            this.writeConcernManager.resolveWriteConcernPromise(result.opId, result);
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
        // Check low water mark after ACKs reduce pending count (delegates to BackpressureController)
        if (ackedCount > 0) {
          this.backpressureController.checkLowWaterMark();
        }
        break;
      }

      case 'LOCK_GRANTED': {
        const { requestId, fencingToken } = message.payload;
        this.lockManager.handleLockGranted(requestId, fencingToken);
        break;
      }

      case 'LOCK_RELEASED': {
        const { requestId, success } = message.payload;
        this.lockManager.handleLockReleased(requestId, success);
        break;
      }

      case 'QUERY_RESP': {
        const { queryId, results, nextCursor, hasMore, cursorStatus } = message.payload;
        const query = this.queryManager.getQueries().get(queryId);
        if (query) {
          query.onResult(results, 'server');
          // Phase 14.1: Update pagination info
          query.updatePaginationInfo({ nextCursor, hasMore, cursorStatus });
        }
        break;
      }

      case 'QUERY_UPDATE': {
        const { queryId, key, value, type } = message.payload;
        const query = this.queryManager.getQueries().get(queryId);
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
        this.topicManager.handleTopicMessage(topic, data, publisherId, timestamp);
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
        this.counterManager.handleCounterUpdate(name, state);
        break;
      }

      case 'COUNTER_RESPONSE': {
        // Initial counter state response
        const { name, state } = message.payload;
        logger.debug({ name }, 'Received COUNTER_RESPONSE');
        this.counterManager.handleCounterUpdate(name, state);
        break;
      }

      // ============ Entry Processor Message Handlers (Phase 5.03) ============

      case 'ENTRY_PROCESS_RESPONSE': {
        logger.debug({ requestId: message.requestId, success: message.success }, 'Received ENTRY_PROCESS_RESPONSE');
        this.entryProcessorClient.handleEntryProcessResponse(message);
        break;
      }

      case 'ENTRY_PROCESS_BATCH_RESPONSE': {
        logger.debug({ requestId: message.requestId }, 'Received ENTRY_PROCESS_BATCH_RESPONSE');
        this.entryProcessorClient.handleEntryProcessBatchResponse(message);
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
        this.searchClient.handleSearchResponse(message.payload);
        break;
      }

      // ============ Live Search Message Handlers (Phase 11.1b) ============

      case 'SEARCH_UPDATE': {
        logger.debug({
          subscriptionId: message.payload?.subscriptionId,
          key: message.payload?.key,
          type: message.payload?.type
        }, 'Received SEARCH_UPDATE');
        // SEARCH_UPDATE is handled by SearchHandle via emitMessage
        // No additional processing needed here
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
    this.webSocketManager.close();

    // Cancel pending Write Concern promises (delegates to WriteConcernManager)
    this.writeConcernManager.cancelAllWriteConcernPromises(new Error('SyncEngine closed'));

    // Clean up CounterManager (Phase 09b)
    this.counterManager.close();

    // Clean up EntryProcessorClient (Phase 09b)
    this.entryProcessorClient.close(new Error('SyncEngine closed'));

    // Clean up SearchClient (Phase 09b)
    this.searchClient.close(new Error('SyncEngine closed'));

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
    const mapKeys = allKeys.filter(k => k.startsWith(mapName + ':'));
    for (const key of mapKeys) {
      await this.storageAdapter.remove(key);
    }
    logger.info({ mapName, removedStorageCount: mapKeys.length }, 'Reset map: Cleared memory and storage');
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

  // ============ Backpressure Methods (delegated to BackpressureController) ============

  /**
   * Get the current number of pending (unsynced) operations.
   * Delegates to BackpressureController.
   */
  public getPendingOpsCount(): number {
    return this.backpressureController.getPendingOpsCount();
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
   * Subscribe to backpressure events.
   * Delegates to BackpressureController.
   * @param event Event name: 'backpressure:high', 'backpressure:low', 'backpressure:paused', 'backpressure:resumed', 'operation:dropped'
   * @param listener Callback function
   * @returns Unsubscribe function
   */
  public onBackpressure(
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    listener: (data?: BackpressureThresholdEvent | OperationDroppedEvent) => void
  ): () => void {
    return this.backpressureController.onBackpressure(event, listener);
  }

  // ============================================
  // Write Concern Methods (Phase 5.01)
  // ============================================

  /**
   * Register a pending Write Concern promise for an operation.
   * Delegates to WriteConcernManager.
   *
   * @param opId - Operation ID
   * @param timeout - Timeout in ms (default: 5000)
   * @returns Promise that resolves with the Write Concern result
   */
  public registerWriteConcernPromise(opId: string, timeout: number = 5000): Promise<any> {
    return this.writeConcernManager.registerWriteConcernPromise(opId, timeout);
  }

  // ============================================
  // PN Counter Methods (Phase 5.2) - Delegates to CounterManager
  // ============================================

  /**
   * Subscribe to counter updates from server.
   * Delegates to CounterManager.
   * @param name Counter name
   * @param listener Callback when counter state is updated
   * @returns Unsubscribe function
   */
  public onCounterUpdate(name: string, listener: (state: any) => void): () => void {
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
  public syncCounter(name: string, state: any): void {
    this.counterManager.syncCounter(name, state);
  }

  // ============================================
  // Entry Processor Methods (Phase 5.03) - Delegates to EntryProcessorClient
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
  // Full-Text Search Methods (Phase 11.1a) - Delegates to SearchClient
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
    options?: SearchOptions
  ): Promise<SearchResult<T>[]> {
    return this.searchClient.search<T>(mapName, query, options);
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

  // ============================================
  // Hybrid Query Support (Phase 12) - Delegates to QueryManager
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
    filter: HybridQueryFilter
  ): Promise<Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>> {
    return this.queryManager.runLocalHybridQuery<T>(mapName, filter);
  }

  /**
   * Handle hybrid query response from server.
   */
  public handleHybridQueryResponse(payload: {
    subscriptionId: string;
    results: Array<{ key: string; value: unknown; score: number; matchedTerms: string[] }>;
    nextCursor?: string;
    hasMore?: boolean;
    cursorStatus?: 'valid' | 'expired' | 'invalid' | 'none';
  }): void {
    const query = this.queryManager.getHybridQuery(payload.subscriptionId);
    if (query) {
      query.onResult(payload.results as any, 'server');
      // Phase 14.1: Update pagination info
      query.updatePaginationInfo({
        nextCursor: payload.nextCursor,
        hasMore: payload.hasMore,
        cursorStatus: payload.cursorStatus,
      });
    }
  }

  /**
   * Handle hybrid query delta update from server.
   */
  public handleHybridQueryDelta(payload: {
    subscriptionId: string;
    key: string;
    value: unknown | null;
    score?: number;
    matchedTerms?: string[];
    type: 'ENTER' | 'UPDATE' | 'LEAVE';
  }): void {
    const query = this.queryManager.getHybridQuery(payload.subscriptionId);
    if (query) {
      if (payload.type === 'LEAVE') {
        query.onUpdate(payload.key, null);
      } else {
        query.onUpdate(payload.key, payload.value, payload.score, payload.matchedTerms);
      }
    }
  }
}
