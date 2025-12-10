import { HLC, LWWMap, ORMap, serialize, deserialize, evaluatePredicate } from '@topgunbuild/core';
import type { LWWRecord, ORMapRecord, Timestamp } from '@topgunbuild/core';
import type { IStorageAdapter } from './IStorageAdapter';
import { QueryHandle } from './QueryHandle';
import type { QueryFilter } from './QueryHandle';
import { TopicHandle } from './TopicHandle';
import { logger } from './utils/logger';

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

export interface SyncEngineConfig {
  nodeId: string;
  serverUrl: string;
  storageAdapter: IStorageAdapter;
  reconnectInterval?: number;
}

export class SyncEngine {
  private readonly nodeId: string;
  private readonly serverUrl: string;
  private readonly storageAdapter: IStorageAdapter;
  private readonly reconnectInterval: number;
  private readonly hlc: HLC;
  private websocket: WebSocket | null = null;
  private isOnline: boolean = false;
  private isAuthenticated: boolean = false;
  private opLog: OpLogEntry[] = [];
  private maps: Map<string, LWWMap<any, any> | ORMap<any, any>> = new Map();
  private queries: Map<string, QueryHandle<any>> = new Map();
  private topics: Map<string, TopicHandle> = new Map();
  private pendingLockRequests: Map<string, { resolve: (res: any) => void, reject: (err: any) => void, timer: any }> = new Map();
  private lastSyncTimestamp: number = 0;
  private reconnectTimer: any = null; // NodeJS.Timeout
  private authToken: string | null = null;
  private tokenProvider: (() => Promise<string | null>) | null = null;

  constructor(config: SyncEngineConfig) {
    this.nodeId = config.nodeId;
    this.serverUrl = config.serverUrl;
    this.storageAdapter = config.storageAdapter;
    this.reconnectInterval = config.reconnectInterval || 5000;
    this.hlc = new HLC(this.nodeId);

    this.initConnection();
    this.loadOpLog();
  }

  private initConnection(): void {
    this.websocket = new WebSocket(this.serverUrl);
    this.websocket.binaryType = 'arraybuffer';

    this.websocket.onopen = () => {
      // [CHANGE] Don't send auth immediately if we don't have a token
      // This prevents the "AUTH_REQUIRED -> Close -> Retry loop" for anonymous initial connects
      if (this.authToken || this.tokenProvider) {
        logger.info('WebSocket connected. Sending auth...');
        this.isOnline = true;
        this.sendAuth();
      } else {
        logger.info('WebSocket connected. Waiting for auth token...');
        // We stay connected but don't send anything until setAuthToken is called
        this.isOnline = true;
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
      logger.info('WebSocket disconnected. Retrying...');
      this.isOnline = false;
      this.isAuthenticated = false;
      this.scheduleReconnect();
    };

    this.websocket.onerror = (error) => {
      logger.error({ err: error }, 'WebSocket error');
    };
  }

  private scheduleReconnect() {
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = setTimeout(() => {
      this.initConnection();
    }, this.reconnectInterval);
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
  ): Promise<void> {

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

    if (this.isOnline && this.isAuthenticated) {
      this.syncPendingOperations();
    }
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

    if (this.isOnline) {
      // If we are already connected (e.g. waiting for token), send it now
      this.sendAuth();
    } else {
      // [CHANGE] Force immediate reconnect if we were waiting for retry timer
      if (this.reconnectTimer) {
        logger.info('Auth token set during backoff. Reconnecting immediately.');
        clearTimeout(this.reconnectTimer);
        this.reconnectTimer = null;
        this.initConnection();
      }
    }
  }

  public setTokenProvider(provider: () => Promise<string | null>): void {
    this.tokenProvider = provider;
    if (this.isOnline && !this.isAuthenticated) {
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
    if (this.isOnline && this.isAuthenticated) {
      this.sendQuerySubscription(query);
    }
  }

  public subscribeToTopic(topic: string, handle: TopicHandle) {
    this.topics.set(topic, handle);
    if (this.isOnline && this.isAuthenticated) {
      this.sendTopicSubscription(topic);
    }
  }

  public unsubscribeFromTopic(topic: string) {
    this.topics.delete(topic);
    if (this.isOnline && this.isAuthenticated) {
      this.websocket?.send(serialize({
        type: 'TOPIC_UNSUB',
        payload: { topic }
      }));
    }
  }

  public publishTopic(topic: string, data: any) {
    if (this.isOnline && this.isAuthenticated) {
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
    if (this.isOnline && this.isAuthenticated) {
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
    if (!this.isOnline || !this.isAuthenticated) {
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
    if (!this.isOnline) return Promise.resolve(false);

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
        const wasAuthenticated = this.isAuthenticated;
        this.isAuthenticated = true;
        this.syncPendingOperations();

        // Only re-subscribe on first authentication to prevent UI flickering
        if (!wasAuthenticated) {
          this.startMerkleSync();
          for (const query of this.queries.values()) {
            this.sendQuerySubscription(query);
          }
          for (const topic of this.topics.keys()) {
            this.sendTopicSubscription(topic);
          }
        }
        break;
      }

      case 'AUTH_FAIL':
        logger.error({ error: message.error }, 'Authentication failed');
        this.isAuthenticated = false;
        this.authToken = null; // Clear invalid token
        break;

      case 'OP_ACK': {
        const { lastId } = message.payload;
        logger.info({ lastId }, 'Received ACK for ops');
        let maxSyncedId = -1;
        this.opLog.forEach(op => {
          if (op.id && op.id <= lastId) {
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
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.websocket) {
      this.websocket.onclose = null; // Prevent reconnect on intentional close
      this.websocket.close();
      this.websocket = null;
    }

    this.isOnline = false;
    this.isAuthenticated = false;
    logger.info('SyncEngine closed');
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
}
