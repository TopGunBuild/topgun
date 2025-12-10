import { HLC, Timestamp } from './HLC';

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

  constructor(hlc: HLC) {
    this.hlc = hlc;
    this.items = new Map();
    this.tombstones = new Set();
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

    this.notify();
    return tagsToRemove;
  }

  /**
   * Clears all data and tombstones.
   */
  public clear(): void {
    this.items.clear();
    this.tombstones.clear();
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
    const now = Date.now();

    for (const [tag, record] of keyMap.entries()) {
      if (!this.tombstones.has(tag)) {
        // Check expiration
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
    const now = Date.now();

    for (const [tag, record] of keyMap.entries()) {
      if (!this.tombstones.has(tag)) {
        // Check expiration
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
   */
  public apply(key: K, record: ORMapRecord<V>): void {
    if (this.tombstones.has(record.tag)) return;

    let keyMap = this.items.get(key);
    if (!keyMap) {
      keyMap = new Map();
      this.items.set(key, keyMap);
    }
    keyMap.set(record.tag, record);
    this.hlc.update(record.timestamp);
    this.notify();
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
        }
      }
      if (localKeyMap.size === 0) {
        this.items.delete(key);
      }
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
}
