import { LWWMap, ORMap } from '@topgunbuild/core';
import type { ORMapRecord, LWWRecord } from '@topgunbuild/core';
import type { IStorageAdapter } from './IStorageAdapter';
import { SyncEngine } from './SyncEngine';
import type { BackoffConfig } from './SyncEngine';
import { QueryHandle } from './QueryHandle';
import type { QueryFilter } from './QueryHandle';
import { DistributedLock } from './DistributedLock';
import { TopicHandle } from './TopicHandle';
import { logger } from './utils/logger';
import { SyncState } from './SyncState';
import type { StateChangeEvent } from './SyncStateMachine';
import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from './BackpressureConfig';

export class TopGunClient {
  private readonly nodeId: string;
  private readonly syncEngine: SyncEngine;
  private readonly maps: Map<string, LWWMap<any, any> | ORMap<any, any>> = new Map();
  private readonly storageAdapter: IStorageAdapter;
  private readonly topicHandles: Map<string, TopicHandle> = new Map();

  constructor(config: {
    nodeId?: string;
    serverUrl: string;
    storage: IStorageAdapter;
    backoff?: Partial<BackoffConfig>;
    backpressure?: Partial<BackpressureConfig>;
  }) {
    this.nodeId = config.nodeId || crypto.randomUUID();
    this.storageAdapter = config.storage;

    const syncEngineConfig = {
      nodeId: this.nodeId,
      serverUrl: config.serverUrl,
      storageAdapter: this.storageAdapter,
      backoff: config.backoff,
      backpressure: config.backpressure,
    };
    this.syncEngine = new SyncEngine(syncEngineConfig);
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
   */
  public query<T>(mapName: string, filter: QueryFilter): QueryHandle<T> {
    return new QueryHandle<T>(this.syncEngine, mapName, filter);
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
   * Retrieves an LWWMap instance. If the map doesn't exist locally, it's created.
   * @param name The name of the map.
   * @returns An LWWMap instance.
   */
  public getMap<K, V>(name: string): LWWMap<K, V> {
    if (this.maps.has(name)) {
      const map = this.maps.get(name);
      if (map instanceof LWWMap) {
        return map as LWWMap<K, V>;
      }
      throw new Error(`Map ${name} exists but is not an LWWMap`);
    }

    const lwwMap = new LWWMap<K, V>(this.syncEngine.getHLC());
    this.maps.set(name, lwwMap);
    this.syncEngine.registerMap(name, lwwMap);

    // Restore state from storage asynchronously
    this.storageAdapter.getAllKeys().then(async (keys) => {
      const mapPrefix = `${name}:`;
      for (const fullKey of keys) {
        if (fullKey.startsWith(mapPrefix)) {
          const record = await this.storageAdapter.get(fullKey);
          if (record && (record as LWWRecord<V>).timestamp && !(record as any).tag) {
            // Strip prefix to get actual key
            const key = fullKey.substring(mapPrefix.length) as unknown as K;
            // Merge into in-memory map without triggering new ops
            lwwMap.merge(key, record as LWWRecord<V>);
          }
        }
      }
    }).catch(err => logger.error({ err }, 'Failed to restore keys from storage'));

    // Wrap LWWMap with IMap interface logic
    const originalSet = lwwMap.set.bind(lwwMap);
    lwwMap.set = (key: K, value: V, ttlMs?: number) => {
      const record = originalSet(key, value, ttlMs);
      this.storageAdapter.put(`${name}:${key}`, record).catch(err => logger.error({ err }, 'Failed to put record to storage'));
      this.syncEngine.recordOperation(name, 'PUT', String(key), { record, timestamp: record.timestamp }).catch(err => logger.error({ err }, 'Failed to record PUT op'));
      return record;
    };

    const originalRemove = lwwMap.remove.bind(lwwMap);
    lwwMap.remove = (key: K) => {
      const tombstone = originalRemove(key);
      this.storageAdapter.put(`${name}:${key}`, tombstone).catch(err => logger.error({ err }, 'Failed to put tombstone to storage'));
      this.syncEngine.recordOperation(name, 'REMOVE', String(key), { record: tombstone, timestamp: tombstone.timestamp }).catch(err => logger.error({ err }, 'Failed to record REMOVE op'));
      return tombstone;
    };

    return lwwMap;
  }

  /**
   * Retrieves an ORMap instance. If the map doesn't exist locally, it's created.
   * @param name The name of the map.
   * @returns An ORMap instance.
   */
  public getORMap<K, V>(name: string): ORMap<K, V> {
    if (this.maps.has(name)) {
      const map = this.maps.get(name);
      if (map instanceof ORMap) {
        return map as ORMap<K, V>;
      }
      throw new Error(`Map ${name} exists but is not an ORMap`);
    }

    const orMap = new ORMap<K, V>(this.syncEngine.getHLC());
    this.maps.set(name, orMap);
    this.syncEngine.registerMap(name, orMap);

    // Restore state from storage
    this.restoreORMap(name, orMap);

    // Wrap ORMap methods to record operations
    const originalAdd = orMap.add.bind(orMap);
    orMap.add = (key: K, value: V, ttlMs?: number) => {
      const record = originalAdd(key, value, ttlMs);
      
      // Persist records
      this.persistORMapKey(name, orMap, key);

      this.syncEngine.recordOperation(name, 'OR_ADD', String(key), { orRecord: record, timestamp: record.timestamp }).catch(err => logger.error({ err }, 'Failed to record OR_ADD op'));
      return record;
    };

    const originalRemove = orMap.remove.bind(orMap);
    orMap.remove = (key: K, value: V) => {
      const tombstones = originalRemove(key, value);
      const timestamp = this.syncEngine.getHLC().now(); 
      
      // Update storage for the key (items removed)
      this.persistORMapKey(name, orMap, key);
      // Update storage for tombstones
      this.persistORMapTombstones(name, orMap);

      for (const tag of tombstones) {
          this.syncEngine.recordOperation(name, 'OR_REMOVE', String(key), { orTag: tag, timestamp }).catch(err => logger.error({ err }, 'Failed to record OR_REMOVE op'));
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
          for (const fullKey of keys) {
              if (fullKey.startsWith(mapPrefix)) {
                  const keyPart = fullKey.substring(mapPrefix.length);
                  
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

  private async persistORMapKey<K, V>(mapName: string, orMap: ORMap<K, V>, key: K) {
      const records = orMap.getRecords(key);
      if (records.length > 0) {
          await this.storageAdapter.put(`${mapName}:${key}`, records);
      } else {
          await this.storageAdapter.remove(`${mapName}:${key}`);
      }
  }
  
  private async persistORMapTombstones<K, V>(mapName: string, orMap: ORMap<K, V>) {
      const tombstoneKey = `__sys__:${mapName}:tombstones`;
      const tombstones = orMap.getTombstones();
      await this.storageAdapter.setMeta(tombstoneKey, tombstones);
  }

  /**
   * Closes the client, disconnecting from the server and cleaning up resources.
   */
  public close(): void {
    this.syncEngine.close();
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
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    listener: (data?: BackpressureThresholdEvent | OperationDroppedEvent) => void
  ): () => void {
    return this.syncEngine.onBackpressure(event, listener);
  }
}
