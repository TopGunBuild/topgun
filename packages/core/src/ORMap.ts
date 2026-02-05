import { HLC, Timestamp } from './HLC';
import { ORMapMerkleTree } from './ORMapMerkleTree';
import { compareTimestamps } from './ORMapMerkle';

/**
 * A record in the OR-Map (Observed-Remove Map).
 * Represents a single value instance with a unique tag.
 */
export interface ORMapRecord<V> {
  value: V;
  timestamp: Timestamp;
  tag: string; // Unique identifier (UUID + Timestamp)
  ttlMs?: number;
}

/**
 * Result of merging records for a key.
 */
export interface MergeKeyResult {
  added: number;
  updated: number;
}

/**
 * Snapshot of ORMap internal state for Merkle Tree synchronization.
 */
export interface ORMapSnapshot<K, V> {
  items: Map<K, Map<string, ORMapRecord<V>>>;
  tombstones: Set<string>;
}

/**
 * OR-Map (Observed-Remove Map) Implementation.
 * 
 * Acts as a Multimap where each Key holds a Set of Values.
 * Supports concurrent additions to the same key without data loss.
 * 
 * Logic:
 * - Add(K, V): Generates a unique tag. Stores (V, tag) under K.
 * - Remove(K, V): Finds all *currently observed* tags for V under K, and moves them to a Remove Set (Tombstones).
 * - Merge: Union of items minus Union of tombstones.
 */
export class ORMap<K, V> {
  // Key -> Map<Tag, Record>
  // Stores active records.
  private items: Map<K, Map<string, ORMapRecord<V>>>;

  // Set of removed tags (Tombstones).
  private tombstones: Set<string>;

  // Set of expired tags (Local only cache for fast filtering)
  // Note: We don't persist this directly, but rely on filtering.
  // For now, we will just filter on get()

  private readonly hlc: HLC;

  // Merkle Tree for efficient sync
  private merkleTree: ORMapMerkleTree;

  constructor(hlc: HLC) {
    this.hlc = hlc;
    this.items = new Map();
    this.tombstones = new Set();
    this.merkleTree = new ORMapMerkleTree();
  }

  private listeners: Array<() => void> = [];

  public onChange(callback: () => void): () => void {
    this.listeners.push(callback);
    return () => {
      this.listeners = this.listeners.filter(cb => cb !== callback);
    };
  }

  private notify(): void {
    this.listeners.forEach(cb => cb());
  }

  public get size(): number {
    return this.items.size;
  }

  public get totalRecords(): number {
    let count = 0;
    for (const keyMap of this.items.values()) {
      count += keyMap.size;
    }
    return count;
  }

  /**
   * Adds a value to the set associated with the key.
   * Generates a unique tag for this specific addition.
   */
  public add(key: K, value: V, ttlMs?: number): ORMapRecord<V> {
    const timestamp = this.hlc.now();
    // Tag must be unique globally. HLC.toString() provides unique string per node+time.
    const tag = HLC.toString(timestamp);

    const record: ORMapRecord<V> = {
      value,
      timestamp,
      tag
    };

    if (ttlMs !== undefined) {
      if (typeof ttlMs !== 'number' || ttlMs <= 0 || !Number.isFinite(ttlMs)) {
        throw new Error('TTL must be a positive finite number');
      }
      record.ttlMs = ttlMs;
    }

    let keyMap = this.items.get(key);
    if (!keyMap) {
      keyMap = new Map();
      this.items.set(key, keyMap);
    }

    keyMap.set(tag, record);
    this.updateMerkleTree(key);
    this.notify();
    return record;
  }

  /**
   * Removes a specific value from the set associated with the key.
   * Marks all *currently observed* instances of this value as removed (tombstones).
   * Returns the list of tags that were removed (useful for sync).
   */
  public remove(key: K, value: V): string[] {
    const keyMap = this.items.get(key);
    if (!keyMap) return [];

    // Find all tags for this value
    const tagsToRemove: string[] = [];

    for (const [tag, record] of keyMap.entries()) {
      // Using strict equality. For objects, this requires the exact instance.
      if (record.value === value) {
        tagsToRemove.push(tag);
      }
    }

    for (const tag of tagsToRemove) {
      this.tombstones.add(tag);
      keyMap.delete(tag);
    }

    if (keyMap.size === 0) {
      this.items.delete(key);
    }

    this.updateMerkleTree(key);
    this.notify();
    return tagsToRemove;
  }

  /**
   * Clears all data and tombstones.
   */
  public clear(): void {
    this.items.clear();
    this.tombstones.clear();
    this.merkleTree = new ORMapMerkleTree();
    this.notify();
  }

  /**
   * Returns all active values for a key.
   * Filters out expired records.
   */
  public get(key: K): V[] {
    const keyMap = this.items.get(key);
    if (!keyMap) return [];

    const values: V[] = [];
    const now = this.hlc.getClockSource().now();

    for (const [tag, record] of keyMap.entries()) {
      if (!this.tombstones.has(tag)) {
        // Check expiration using HLC's clock source
        if (record.ttlMs && record.timestamp.millis + record.ttlMs < now) {
          continue;
        }
        values.push(record.value);
      }
    }
    return values;
  }

  /**
   * Returns all active records for a key.
   * Useful for persistence and sync.
   * Filters out expired records.
   */
  public getRecords(key: K): ORMapRecord<V>[] {
    const keyMap = this.items.get(key);
    if (!keyMap) return [];

    const records: ORMapRecord<V>[] = [];
    const now = this.hlc.getClockSource().now();

    for (const [tag, record] of keyMap.entries()) {
      if (!this.tombstones.has(tag)) {
        // Check expiration using HLC's clock source
        if (record.ttlMs && record.timestamp.millis + record.ttlMs < now) {
          continue;
        }
        records.push(record);
      }
    }
    return records;
  }

  /**
   * Returns all tombstone tags.
   */
  public getTombstones(): string[] {
    return Array.from(this.tombstones);
  }

  /**
   * Applies a record from a remote source (Sync).
   * Returns true if the record was applied (not tombstoned).
   */
  public apply(key: K, record: ORMapRecord<V>): boolean {
    if (this.tombstones.has(record.tag)) return false;

    let keyMap = this.items.get(key);
    if (!keyMap) {
      keyMap = new Map();
      this.items.set(key, keyMap);
    }
    keyMap.set(record.tag, record);
    this.hlc.update(record.timestamp);
    this.updateMerkleTree(key);
    this.notify();
    return true;
  }

  /**
   * Applies a tombstone (deletion) from a remote source.
   */
  public applyTombstone(tag: string): void {
    this.tombstones.add(tag);
    // Cleanup active items if present
    for (const [key, keyMap] of this.items) {
      if (keyMap.has(tag)) {
        keyMap.delete(tag);
        if (keyMap.size === 0) this.items.delete(key);
        this.updateMerkleTree(key);
        // We found it, so we can stop searching (tag is unique globally)
        break;
      }
    }
    this.notify();
  }

  /**
   * Merges state from another ORMap.
   * - Adds all new tombstones from 'other'.
   * - Adds all new items from 'other' that are not in tombstones.
   * - Updates HLC with observed timestamps.
   */
  public merge(other: ORMap<K, V>): void {
    const changedKeys = new Set<K>();

    // 1. Merge tombstones
    for (const tag of other.tombstones) {
      this.tombstones.add(tag);
    }

    // 2. Merge items
    for (const [key, otherKeyMap] of other.items) {
      let localKeyMap = this.items.get(key);
      if (!localKeyMap) {
        localKeyMap = new Map();
        this.items.set(key, localKeyMap);
      }

      for (const [tag, record] of otherKeyMap) {
        // Only accept if not deleted
        if (!this.tombstones.has(tag)) {
          if (!localKeyMap.has(tag)) {
            localKeyMap.set(tag, record);
            changedKeys.add(key);
          }
          // Always update causality
          this.hlc.update(record.timestamp);
        }
      }
    }

    // 3. Cleanup: Remove any local items that are now in the merged tombstones
    for (const [key, localKeyMap] of this.items) {
      for (const tag of localKeyMap.keys()) {
        if (this.tombstones.has(tag)) {
          localKeyMap.delete(tag);
          changedKeys.add(key);
        }
      }
      if (localKeyMap.size === 0) {
        this.items.delete(key);
      }
    }

    // Update Merkle Tree for changed keys
    for (const key of changedKeys) {
      this.updateMerkleTree(key);
    }

    this.notify();
  }

  /**
   * Garbage Collection: Prunes tombstones older than the specified timestamp.
   */
  public prune(olderThan: Timestamp): string[] {
    const removedTags: string[] = [];

    for (const tag of this.tombstones) {
      try {
        const timestamp = HLC.parse(tag);
        if (HLC.compare(timestamp, olderThan) < 0) {
          this.tombstones.delete(tag);
          removedTags.push(tag);
        }
      } catch (e) {
        // Ignore invalid tags
      }
    }

    return removedTags;
  }

  // ============ Merkle Sync Methods ============

  /**
   * Get the Merkle Tree for this ORMap.
   * Used for efficient synchronization.
   */
  public getMerkleTree(): ORMapMerkleTree {
    return this.merkleTree;
  }

  /**
   * Get a snapshot of internal state for Merkle Tree synchronization.
   * Returns references to internal structures - do not modify!
   */
  public getSnapshot(): ORMapSnapshot<K, V> {
    return {
      items: this.items,
      tombstones: this.tombstones
    };
  }

  /**
   * Get all keys in this ORMap.
   */
  public allKeys(): K[] {
    return Array.from(this.items.keys());
  }

  /**
   * Get the internal records map for a key.
   * Returns Map<tag, record> or undefined if key doesn't exist.
   * Used for Merkle sync.
   */
  public getRecordsMap(key: K): Map<string, ORMapRecord<V>> | undefined {
    return this.items.get(key);
  }

  /**
   * Merge remote records for a specific key into local state.
   * Implements Observed-Remove CRDT semantics.
   * Used during Merkle Tree synchronization.
   *
   * @param key The key to merge
   * @param remoteRecords Array of records from remote
   * @param remoteTombstones Array of tombstone tags from remote
   * @returns Result with count of added and updated records
   */
  public mergeKey(
    key: K,
    remoteRecords: ORMapRecord<V>[],
    remoteTombstones: string[] = []
  ): MergeKeyResult {
    let added = 0;
    let updated = 0;

    // First apply remote tombstones
    for (const tag of remoteTombstones) {
      if (!this.tombstones.has(tag)) {
        this.tombstones.add(tag);
      }
    }

    // Get or create local key map
    let localKeyMap = this.items.get(key);
    if (!localKeyMap) {
      localKeyMap = new Map();
      this.items.set(key, localKeyMap);
    }

    // Remove any local records that are now tombstoned
    for (const tag of localKeyMap.keys()) {
      if (this.tombstones.has(tag)) {
        localKeyMap.delete(tag);
      }
    }

    // Merge remote records
    for (const remoteRecord of remoteRecords) {
      // Skip if tombstoned
      if (this.tombstones.has(remoteRecord.tag)) {
        continue;
      }

      const localRecord = localKeyMap.get(remoteRecord.tag);

      if (!localRecord) {
        // New record - add it
        localKeyMap.set(remoteRecord.tag, remoteRecord);
        added++;
      } else if (compareTimestamps(remoteRecord.timestamp, localRecord.timestamp) > 0) {
        // Remote is newer - update
        localKeyMap.set(remoteRecord.tag, remoteRecord);
        updated++;
      }
      // Else: local is newer or equal, keep local

      // Always update causality
      this.hlc.update(remoteRecord.timestamp);
    }

    // Cleanup empty key map
    if (localKeyMap.size === 0) {
      this.items.delete(key);
    }

    // Update Merkle Tree
    this.updateMerkleTree(key);

    if (added > 0 || updated > 0) {
      this.notify();
    }

    return { added, updated };
  }

  /**
   * Check if a tag is tombstoned.
   */
  public isTombstoned(tag: string): boolean {
    return this.tombstones.has(tag);
  }

  /**
   * Update the Merkle Tree for a specific key.
   * Called internally after any modification.
   */
  private updateMerkleTree(key: K): void {
    const keyStr = String(key);
    const keyMap = this.items.get(key);

    if (!keyMap || keyMap.size === 0) {
      this.merkleTree.remove(keyStr);
    } else {
      this.merkleTree.update(keyStr, keyMap);
    }
  }
}
