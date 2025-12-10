import { HLC, Timestamp } from './HLC';
import { MerkleTree } from './MerkleTree';

/**
 * A record in the LWW-Map.
 * Can represent a value or a deletion (tombstone).
 */
export interface LWWRecord<V> {
  value: V | null;
  timestamp: Timestamp;
  ttlMs?: number;
}

/**
 * Last-Write-Wins Map Implementation.
 * This structure guarantees convergence by always keeping the entry with the highest timestamp.
 */
export class LWWMap<K, V> {
  private data: Map<K, LWWRecord<V>>;
  private readonly hlc: HLC;
  private listeners: Array<() => void> = [];
  private merkleTree: MerkleTree;

  constructor(hlc: HLC) {
    this.hlc = hlc;
    this.data = new Map();
    this.merkleTree = new MerkleTree();
  }

  public onChange(callback: () => void): () => void {
    this.listeners.push(callback);
    return () => {
      this.listeners = this.listeners.filter(cb => cb !== callback);
    };
  }

  private notify(): void {
    this.listeners.forEach(cb => cb());
  }

  public getMerkleTree(): MerkleTree {
    return this.merkleTree;
  }

  public get size(): number {
    return this.data.size;
  }

  /**
   * Sets a value for a key.
   * Generates a new timestamp using the local HLC.
   */
  public set(key: K, value: V, ttlMs?: number): LWWRecord<V> {
    const timestamp = this.hlc.now();
    const record: LWWRecord<V> = { value, timestamp };
    
    if (ttlMs !== undefined) {
      if (typeof ttlMs !== 'number' || ttlMs <= 0 || !Number.isFinite(ttlMs)) {
        // We could throw, but to be resilient we might just ignore invalid TTL or log warning.
        // Given this is core lib, throwing is safer to alert dev.
        throw new Error('TTL must be a positive finite number');
      }
      record.ttlMs = ttlMs;
    }
    
    // We assume K is string for MerkleTree compatibility in this system
    // If K is not string, we might need to stringify it.
    // The project seems to use string keys for maps.
    this.data.set(key, record);
    this.merkleTree.update(String(key), record);
    
    this.notify();
    return record;
  }

  /**
   * Retrieves the value for a key.
   * Returns undefined if key doesn't exist, is a tombstone, or is expired.
   */
  public get(key: K): V | undefined {
    const record = this.data.get(key);
    if (!record || record.value === null) {
      return undefined;
    }

    // Check for expiration
    if (record.ttlMs) {
      const now = Date.now();
      if (record.timestamp.millis + record.ttlMs < now) {
        return undefined;
      }
    }

    return record.value;
  }

  /**
   * Returns the full record (including timestamp).
   * Useful for synchronization.
   */
  public getRecord(key: K): LWWRecord<V> | undefined {
    return this.data.get(key);
  }

  /**
   * Removes a key (creates a tombstone).
   */
  public remove(key: K): LWWRecord<V> {
    const timestamp = this.hlc.now();
    const tombstone: LWWRecord<V> = { value: null, timestamp };
    
    this.data.set(key, tombstone);
    this.merkleTree.update(String(key), tombstone);
    
    this.notify();
    return tombstone;
  }

  /**
   * Merges a record from a remote source.
   * Returns true if the local state was updated.
   */
  public merge(key: K, remoteRecord: LWWRecord<V>): boolean {
    // Update our clock to ensure causality for future events
    this.hlc.update(remoteRecord.timestamp);

    const localRecord = this.data.get(key);

    // LWW Logic:
    // 1. If no local record, accept remote.
    // 2. If remote is strictly greater than local, accept remote.
    // 3. If equal, we can arbitrarily choose (e.g. by NodeID) to ensure convergence, 
    //    but HLC.compare handles nodeId tie-breaking already.
    
    if (!localRecord || HLC.compare(remoteRecord.timestamp, localRecord.timestamp) > 0) {
      this.data.set(key, remoteRecord);
      this.merkleTree.update(String(key), remoteRecord);
      
      this.notify();
      return true;
    }

    return false;
  }

  /**
   * Garbage Collection: Prunes tombstones older than the specified timestamp.
   * Only removes records that are tombstones (deleted) AND older than the threshold.
   * 
   * @param olderThan The timestamp threshold. Tombstones older than this will be removed.
   * @returns The number of tombstones removed.
   */
  public prune(olderThan: Timestamp): K[] {
    const removedKeys: K[] = [];
    
    for (const [key, record] of this.data.entries()) {
      // Only prune tombstones (value === null)
      if (record.value === null) {
        // Check if timestamp is strictly older than the threshold
        // HLC.compare(a, b) returns < 0 if a < b
        if (HLC.compare(record.timestamp, olderThan) < 0) {
          this.data.delete(key);
          this.merkleTree.remove(String(key));
          removedKeys.push(key);
        }
      }
    }

    if (removedKeys.length > 0) {
      this.notify();
    }

    return removedKeys;
  }

  /**
   * Clears all data and tombstones.
   * Resets the MerkleTree.
   */
  public clear(): void {
    this.data.clear();
    this.merkleTree = new MerkleTree();
    this.notify();
  }

  /**
   * Returns an iterator over all non-deleted entries.
   */
  public entries(): IterableIterator<[K, V]> {
    const iterator = this.data.entries();
    const now = Date.now();
    
    return {
      [Symbol.iterator]() { return this; },
      next: () => {
        let result = iterator.next();
        while (!result.done) {
          const [key, record] = result.value;
          if (record.value !== null) {
            // Check TTL
            if (record.ttlMs && record.timestamp.millis + record.ttlMs < now) {
                result = iterator.next();
                continue;
            }
            return { value: [key, record.value], done: false };
          }
          result = iterator.next();
        }
        return { value: undefined, done: true };
      }
    };
  }

  /**
   * Returns all keys (including tombstones).
   */
  public allKeys(): IterableIterator<K> {
    return this.data.keys();
  }
}
