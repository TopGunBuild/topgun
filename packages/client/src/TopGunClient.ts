import { LWWMap, ORMap } from '@topgunbuild/core';
import type { ORMapRecord, LWWRecord, EntryProcessorDef, EntryProcessorResult } from '@topgunbuild/core';
import type { IStorageAdapter } from './IStorageAdapter';
import { SyncEngine } from './SyncEngine';
import type { BackoffConfig } from './SyncEngine';
import { QueryHandle } from './QueryHandle';
import type { QueryFilter } from './QueryHandle';
import { DistributedLock } from './DistributedLock';
import { TopicHandle } from './TopicHandle';
import { PNCounterHandle } from './PNCounterHandle';
import { EventJournalReader } from './EventJournalReader';
import { logger } from './utils/logger';
import { SyncState } from './SyncState';
import type { StateChangeEvent } from './SyncStateMachine';
import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from './BackpressureConfig';
import { ClusterClient } from './cluster/ClusterClient';
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
}

export class TopGunClient {
  private readonly nodeId: string;
  private readonly syncEngine: SyncEngine;
  private readonly maps: Map<string, LWWMap<any, any> | ORMap<any, any>> = new Map();
  private readonly storageAdapter: IStorageAdapter;
  private readonly topicHandles: Map<string, TopicHandle> = new Map();
  private readonly counters: Map<string, PNCounterHandle> = new Map();
  private readonly clusterClient?: ClusterClient;
  private readonly isClusterMode: boolean;
  private readonly clusterConfig?: Required<Omit<TopGunClusterConfig, 'seeds'>> & { seeds: string[] };

  constructor(config: TopGunClientConfig) {
    // Validate: either serverUrl or cluster, not both
    if (config.serverUrl && config.cluster) {
      throw new Error('Cannot specify both serverUrl and cluster config');
    }
    if (!config.serverUrl && !config.cluster) {
      throw new Error('Must specify either serverUrl or cluster config');
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
        connectionsPerNode: config.cluster.connectionsPerNode ?? DEFAULT_CLUSTER_CONFIG.connectionsPerNode,
        smartRouting: config.cluster.smartRouting ?? DEFAULT_CLUSTER_CONFIG.smartRouting,
        partitionMapRefreshMs: config.cluster.partitionMapRefreshMs ?? DEFAULT_CLUSTER_CONFIG.partitionMapRefreshMs,
        connectionTimeoutMs: config.cluster.connectionTimeoutMs ?? DEFAULT_CLUSTER_CONFIG.connectionTimeoutMs,
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
      });

      logger.info({ seeds: this.clusterConfig.seeds }, 'TopGunClient initialized in cluster mode');
    } else {
      // Single-server mode (existing behavior)
      this.syncEngine = new SyncEngine({
        nodeId: this.nodeId,
        serverUrl: config.serverUrl!,
        storageAdapter: this.storageAdapter,
        backoff: config.backoff,
        backpressure: config.backpressure,
      });

      logger.info({ serverUrl: config.serverUrl }, 'TopGunClient initialized in single-server mode');
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
      counter = new PNCounterHandle(name, this.nodeId, this.syncEngine);
      this.counters.set(name, counter);
    }
    return counter;
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
    if (this.clusterClient) {
      this.clusterClient.close();
    }
    this.syncEngine.close();
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

  // ============================================
  // Entry Processor API (Phase 5.03)
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
  public async executeOnKey<V, R = V>(
    mapName: string,
    key: string,
    processor: EntryProcessorDef<V, R>,
  ): Promise<EntryProcessorResult<R>> {
    const result = await this.syncEngine.executeOnKey(mapName, key, processor);

    // Update local map cache if successful and we have the map
    if (result.success && result.newValue !== undefined) {
      const map = this.maps.get(mapName);
      if (map instanceof LWWMap) {
        // Update local cache - set() generates its own timestamp
        // The server will broadcast the full update to all subscribers
        (map as LWWMap<any, any>).set(key, result.newValue);
      }
    }

    return result;
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
  public async executeOnKeys<V, R = V>(
    mapName: string,
    keys: string[],
    processor: EntryProcessorDef<V, R>,
  ): Promise<Map<string, EntryProcessorResult<R>>> {
    const results = await this.syncEngine.executeOnKeys(mapName, keys, processor);

    // Update local map cache for successful operations
    const map = this.maps.get(mapName);
    if (map instanceof LWWMap) {
      for (const [key, result] of results) {
        if (result.success && result.newValue !== undefined) {
          (map as LWWMap<any, any>).set(key, result.newValue);
        }
      }
    }

    return results;
  }

  // ============================================
  // Event Journal API (Phase 5.04)
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
}
