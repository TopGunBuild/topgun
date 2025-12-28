import {
  LWWMap,
  LWWRecord,
  MergeContext,
  MergeResult,
  MergeRejection,
  Timestamp,
  HLC,
} from '@topgunbuild/core';
import { ConflictResolverService } from './ConflictResolverService';

/**
 * Configuration for MapWithResolver.
 */
export interface MapWithResolverConfig {
  /** Map name */
  name: string;

  /** Node ID for HLC */
  nodeId: string;

  /** Conflict resolver service */
  resolverService: ConflictResolverService;

  /** Callback for merge rejections */
  onRejection?: (rejection: MergeRejection) => void;
}

/**
 * Result of setWithResolver operation.
 */
export interface SetWithResolverResult<V> {
  /** Whether the value was applied */
  applied: boolean;

  /** The merge result */
  result: MergeResult<V>;

  /** The final record if applied */
  record?: LWWRecord<V>;
}

/**
 * Extended LWWMap that supports custom conflict resolvers.
 *
 * This wrapper delegates merge operations to ConflictResolverService,
 * allowing custom business logic to intercept and modify merge behavior.
 */
export class MapWithResolver<K extends string, V> {
  private map: LWWMap<K, V>;
  private resolverService: ConflictResolverService;
  private mapName: string;
  private hlc: HLC;
  private onRejection?: (rejection: MergeRejection) => void;

  constructor(config: MapWithResolverConfig) {
    this.mapName = config.name;
    this.hlc = new HLC(config.nodeId);
    this.map = new LWWMap<K, V>(this.hlc);
    this.resolverService = config.resolverService;
    this.onRejection = config.onRejection;
  }

  /**
   * Get the map name.
   */
  get name(): string {
    return this.mapName;
  }

  /**
   * Get the underlying LWWMap.
   */
  get rawMap(): LWWMap<K, V> {
    return this.map;
  }

  /**
   * Get a value by key.
   */
  get(key: K): V | undefined {
    return this.map.get(key);
  }

  /**
   * Get the full record for a key.
   */
  getRecord(key: K): LWWRecord<V> | undefined {
    return this.map.getRecord(key);
  }

  /**
   * Get the timestamp for a key.
   */
  getTimestamp(key: K): Timestamp | undefined {
    return this.map.getRecord(key)?.timestamp;
  }

  /**
   * Set a value locally (no resolver).
   * Use for server-initiated writes.
   */
  set(key: K, value: V, ttlMs?: number): LWWRecord<V> {
    return this.map.set(key, value, ttlMs);
  }

  /**
   * Set a value with conflict resolution.
   * Use for client-initiated writes.
   *
   * @param key The key to set
   * @param value The new value
   * @param timestamp The client's timestamp
   * @param remoteNodeId The client's node ID
   * @param auth Optional authentication context
   * @returns Result containing applied status and merge result
   */
  async setWithResolver(
    key: K,
    value: V,
    timestamp: Timestamp,
    remoteNodeId: string,
    auth?: MergeContext['auth'],
  ): Promise<SetWithResolverResult<V>> {
    // Build merge context
    const context: MergeContext<V> = {
      mapName: this.mapName,
      key,
      localValue: this.map.get(key),
      remoteValue: value,
      localTimestamp: this.getTimestamp(key),
      remoteTimestamp: timestamp,
      remoteNodeId,
      auth,
      readEntry: (k: string) => this.map.get(k as K),
    };

    // Resolve conflict
    const result = await this.resolverService.resolve(context);

    // Apply result
    switch (result.action) {
      case 'accept': {
        // Create record with the accepted value
        const record: LWWRecord<V> = {
          value: result.value,
          timestamp,
        };
        // Use internal merge to properly update HLC and Merkle tree
        this.map.merge(key, record);
        return { applied: true, result, record };
      }

      case 'merge': {
        // Create record with the merged value
        const record: LWWRecord<V> = {
          value: result.value,
          timestamp,
        };
        this.map.merge(key, record);
        return { applied: true, result, record };
      }

      case 'reject': {
        // Emit rejection event
        if (this.onRejection) {
          this.onRejection({
            mapName: this.mapName,
            key,
            attemptedValue: value,
            reason: result.reason,
            timestamp,
            nodeId: remoteNodeId,
          });
        }
        return { applied: false, result };
      }

      case 'local': {
        // Keep current value, don't update
        return { applied: false, result };
      }

      default:
        // Shouldn't happen, but fallback to accept
        const record: LWWRecord<V> = {
          value: (result as MergeResult<V> & { action: 'accept' }).value ?? value,
          timestamp,
        };
        this.map.merge(key, record);
        return { applied: true, result, record };
    }
  }

  /**
   * Remove a key.
   */
  remove(key: K): LWWRecord<V> {
    return this.map.remove(key);
  }

  /**
   * Standard merge without resolver (for sync operations).
   */
  merge(key: K, record: LWWRecord<V>): boolean {
    return this.map.merge(key, record);
  }

  /**
   * Merge with resolver support.
   * Equivalent to setWithResolver but takes a full record.
   */
  async mergeWithResolver(
    key: K,
    record: LWWRecord<V>,
    remoteNodeId: string,
    auth?: MergeContext['auth'],
  ): Promise<SetWithResolverResult<V>> {
    if (record.value === null) {
      // Tombstone - apply directly without resolver
      const applied = this.map.merge(key, record);
      return {
        applied,
        result: applied
          ? { action: 'accept', value: record.value as V }
          : { action: 'local' },
        record: applied ? record : undefined,
      };
    }

    return this.setWithResolver(
      key,
      record.value,
      record.timestamp,
      remoteNodeId,
      auth,
    );
  }

  /**
   * Clear all data.
   */
  clear(): void {
    this.map.clear();
  }

  /**
   * Get map size.
   */
  get size(): number {
    return this.map.size;
  }

  /**
   * Iterate over entries.
   */
  entries(): IterableIterator<[K, V]> {
    return this.map.entries();
  }

  /**
   * Get all keys.
   */
  allKeys(): IterableIterator<K> {
    return this.map.allKeys();
  }

  /**
   * Subscribe to changes.
   */
  onChange(callback: () => void): () => void {
    return this.map.onChange(callback);
  }

  /**
   * Get MerkleTree for sync.
   */
  getMerkleTree() {
    return this.map.getMerkleTree();
  }

  /**
   * Prune old tombstones.
   */
  prune(olderThan: Timestamp): K[] {
    return this.map.prune(olderThan);
  }
}
