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
  /**
   * Tombstone tags attributed to each key. Every tag here is also in
   * `tombstones`; a key may appear here without appearing in `items`.
   */
  keyTombstones: Map<K, Set<string>>;
}

/**
 * Whether two map keys are the same key as a `Map` sees them (SameValueZero),
 * so a NaN key is recognised as itself.
 */
function sameKey<K>(a: K, b: K): boolean {
  return a === b || (a !== a && b !== b);
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
  // Map-wide: a tag in here is suppressed under every key (remove-wins).
  private tombstones: Set<string>;

  // Key -> tombstone tags attributed to that key.
  // An overlay on top of the map-wide set, never a replacement for it: it
  // decides nothing about which records are visible. It exists because the
  // server keeps tombstones per key and hashes them into that key's Merkle
  // leaf (TG-MRK-001), so the client needs the same per-key set to compute a
  // comparable leaf. Every tag in here is also in `tombstones`. Tombstones
  // whose key is unknown stay in the map-wide set only.
  private keyTombstones: Map<K, Set<string>>;

  // Tag -> the key holding a live record with that tag: the reverse of `items`.
  // Tombstoning a tag has to drop its record wherever it lives, and a tombstone
  // tag is normally live nowhere, so without this every such lookup would walk
  // all keys only to find nothing. Remove-wins depends on it being exact: a
  // record missing from here survives the tombstone that names its tag. Records
  // therefore enter and leave `items` only through `putRecord` / `dropRecord`
  // (and `purgeLiveTags` / `clear`), which keep the two in step.
  private liveTagKey: Map<string, K>;

  // Further keys holding a record with a tag that `liveTagKey` already maps.
  // A tag is unique by construction (one HLC timestamp per add), so this stays
  // empty in a healthy map. Nothing enforces that uniqueness on the way in,
  // though: `apply`, `mergeKey` and `merge` take a peer's tag under whichever
  // key the peer names. Tracking the extra holders keeps the index exact for
  // such input instead of silently losing sight of one of the records, without
  // paying for a Set per record in the ordinary single-holder case.
  private liveTagExtraKeys: Map<string, Set<K>>;

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
    this.keyTombstones = new Map();
    this.liveTagKey = new Map();
    this.liveTagExtraKeys = new Map();
    this.merkleTree = new ORMapMerkleTree();
  }

  private listeners: Array<(entries: Array<[K, V[]]>) => void> = [];

  public subscribe(callback: (entries: Array<[K, V[]]>) => void): () => void {
    this.listeners.push(callback);
    return () => {
      this.listeners = this.listeners.filter((cb) => cb !== callback);
    };
  }

  private notify(): void {
    if (this.listeners.length === 0) return;
    const snapshot: Array<[K, V[]]> = [];
    for (const key of this.allKeys()) {
      const values = this.get(key);
      if (values.length > 0) {
        snapshot.push([key, values]);
      }
    }
    this.listeners.forEach((cb) => cb(snapshot));
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
      tag,
    };

    if (ttlMs !== undefined) {
      if (typeof ttlMs !== 'number' || ttlMs <= 0 || !Number.isFinite(ttlMs)) {
        throw new Error('TTL must be a positive finite number');
      }
      record.ttlMs = ttlMs;
    }

    this.putRecord(key, this.keyMapFor(key), record);
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
      this.dropRecord(key, keyMap, tag);
      // The remove was observed under this key, so the tombstone belongs to it.
      this.attribute(key, tag);
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
    this.liveTagKey.clear();
    this.liveTagExtraKeys.clear();
    this.tombstones.clear();
    this.keyTombstones.clear();
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

    this.putRecord(key, this.keyMapFor(key), record);
    this.hlc.update(record.timestamp);
    this.updateMerkleTree(key);
    this.notify();
    return true;
  }

  /**
   * Tombstone tags attributed to `key`.
   *
   * Returns a copy, so the caller cannot break the "attributed implies
   * tombstoned map-wide" relation by mutating the result.
   */
  public getKeyTombstones(key: K): Set<string> {
    return new Set(this.keyTombstones.get(key));
  }

  /**
   * Makes `tags` the complete set of tombstones attributed to `key`.
   *
   * This is a REPLACE of the key's attribution, used to mirror the per-key set
   * an authoritative peer reports. An empty `tags` clears the attribution, and
   * the key then leaves the Merkle tree unless it still holds records.
   *
   * Every tag also goes into the map-wide tombstone set, which only ever grows
   * here: a tag dropped from this key's attribution stays suppressed. A live
   * record carrying a newly attributed tag is removed, under whichever key it
   * lives, exactly as `applyTombstone` would remove it.
   *
   * Subscribers are notified at most once per call, and only when a live record
   * was actually removed. A sync walk calls this for every key it visits and
   * most of those calls change nothing a subscriber can see.
   *
   * The cost is proportional to the number of tags passed, whatever the size
   * of the map.
   */
  public setKeyTombstones(key: K, tags: Iterable<string>): void {
    const next = new Set(tags);
    const previous = this.keyTombstones.get(key);

    // Only tags new to this key can still have a live record: a tag attributed
    // earlier was purged when it was attributed, and no later write re-admits a
    // tombstoned tag.
    const added = new Set<string>();
    for (const tag of next) {
      this.tombstones.add(tag);
      if (!previous || !previous.has(tag)) added.add(tag);
    }

    const purged = this.purgeLiveTags(added);

    if (next.size === 0) {
      this.keyTombstones.delete(key);
    } else {
      this.keyTombstones.set(key, next);
    }

    this.updateMerkleTree(key);
    const purgedKeys = new Set<K>();
    for (const [purgedKey] of purged) {
      if (purgedKey !== key) purgedKeys.add(purgedKey);
    }
    for (const purgedKey of purgedKeys) {
      this.updateMerkleTree(purgedKey);
    }

    if (purged.length > 0) {
      this.notify();
    }
  }

  /**
   * Attributes further tombstones to many keys in one pass.
   *
   * Unlike `setKeyTombstones` this is a UNION per key: the given tags are added
   * to whatever the key already has attributed, and nothing is dropped. The
   * outcome is the one a `setKeyTombstones(key, existing + tags)` call per
   * entry would produce, but the live records are purged once for all the
   * newly attributed tags together, each affected key's Merkle leaf is hashed
   * once, and subscribers are notified at most once (only when a live record
   * was actually removed).
   *
   * Meant for re-applying a stored attribution while a map is loaded, where
   * every key of the map arrives in a single batch. An entry with no tags
   * attributes nothing and leaves no trace of its key.
   *
   * @internal
   */
  public addKeyTombstones(entries: Iterable<readonly [K, Iterable<string>]>): void {
    const added = new Set<string>();
    const changedKeys = new Set<K>();

    for (const [key, tags] of entries) {
      let attributed = this.keyTombstones.get(key);
      const sizeBefore = attributed?.size ?? 0;

      for (const tag of tags) {
        // Created on the first tag only: an empty set must never be stored,
        // or the key would look attributed while having no leaf.
        if (!attributed) {
          attributed = new Set();
          this.keyTombstones.set(key, attributed);
        }
        const size = attributed.size;
        attributed.add(tag);
        // As in `setKeyTombstones`, only a tag new to the key can still have
        // a live record; one attributed earlier is already suppressed.
        if (attributed.size !== size) {
          this.tombstones.add(tag);
          added.add(tag);
        }
      }

      if (attributed && attributed.size !== sizeBefore) changedKeys.add(key);
    }

    const purged = this.purgeLiveTags(added);
    for (const [purgedKey] of purged) {
      changedKeys.add(purgedKey);
    }
    for (const key of changedKeys) {
      this.updateMerkleTree(key);
    }

    if (purged.length > 0) {
      this.notify();
    }
  }

  /**
   * Removes the live records carrying any of `tags`, under every key, and
   * returns what it removed. Does not touch the Merkle tree or subscribers.
   *
   * Costs one index lookup per tag, not a walk over the map: a tag that is
   * live nowhere (the usual case for a tombstone) is dismissed immediately.
   *
   * Protected so that a subclass keeping derived state per record (indexes)
   * can observe every removal a tombstone causes in one place.
   */
  protected purgeLiveTags(tags: Iterable<string>): Array<[K, ORMapRecord<V>]> {
    const purged: Array<[K, ORMapRecord<V>]> = [];

    for (const tag of tags) {
      if (!this.liveTagKey.has(tag)) continue;

      // The tag leaves the index as a whole, so the holders are collected
      // first and their records deleted without going back through it.
      const holders: K[] = [this.liveTagKey.get(tag) as K];
      const extraKeys = this.liveTagExtraKeys.get(tag);
      if (extraKeys) holders.push(...extraKeys);
      this.liveTagKey.delete(tag);
      this.liveTagExtraKeys.delete(tag);

      for (const holder of holders) {
        const keyMap = this.items.get(holder);
        const record = keyMap?.get(tag);
        if (!keyMap || !record) continue;
        keyMap.delete(tag);
        if (keyMap.size === 0) this.items.delete(holder);
        purged.push([holder, record]);
      }
    }
    return purged;
  }

  /**
   * Applies a tombstone (deletion) from a remote source.
   *
   * `key` names the key the tombstone belongs to. When it is given, the tag is
   * also attributed to that key and so enters the key's Merkle leaf. Without
   * it the tombstone is recorded map-wide only: it suppresses the tag exactly
   * the same, but belongs to no key's leaf (the form used for tombstones whose
   * key was never recorded).
   */
  public applyTombstone(tag: string, key?: K): void {
    this.tombstones.add(tag);
    // Drop the record carrying the tag, wherever it lives.
    const changedKeys = new Set<K>();
    for (const [itemKey] of this.purgeLiveTags([tag])) {
      changedKeys.add(itemKey);
    }
    if (key !== undefined) {
      this.attribute(key, tag);
      changedKeys.add(key);
    }
    for (const changedKey of changedKeys) {
      this.updateMerkleTree(changedKey);
    }
    this.notify();
  }

  /** Adds `tag` to the tombstones attributed to `key`. The caller updates the tree. */
  private attribute(key: K, tag: string): void {
    let attributed = this.keyTombstones.get(key);
    if (!attributed) {
      attributed = new Set();
      this.keyTombstones.set(key, attributed);
    }
    attributed.add(tag);
  }

  /** The record map of `key`, created empty when the key holds nothing yet. */
  private keyMapFor(key: K): Map<string, ORMapRecord<V>> {
    let keyMap = this.items.get(key);
    if (!keyMap) {
      keyMap = new Map();
      this.items.set(key, keyMap);
    }
    return keyMap;
  }

  /**
   * Stores `record` under `key` and indexes its tag. `keyMap` is the key's
   * record map. Replacing a record that has the same tag is fine: the index
   * entry is already there.
   */
  private putRecord(key: K, keyMap: Map<string, ORMapRecord<V>>, record: ORMapRecord<V>): void {
    const tag = record.tag;
    keyMap.set(tag, record);

    if (!this.liveTagKey.has(tag)) {
      this.liveTagKey.set(tag, key);
      return;
    }
    if (sameKey(this.liveTagKey.get(tag) as K, key)) return;

    let extraKeys = this.liveTagExtraKeys.get(tag);
    if (!extraKeys) {
      extraKeys = new Set();
      this.liveTagExtraKeys.set(tag, extraKeys);
    }
    extraKeys.add(key);
  }

  /**
   * Deletes the record with `tag` from under `key` and from the index. Leaves
   * an emptied `keyMap` in `items`; the caller drops it once it is done with
   * the key.
   */
  private dropRecord(key: K, keyMap: Map<string, ORMapRecord<V>>, tag: string): void {
    if (!keyMap.delete(tag)) return;

    const extraKeys = this.liveTagExtraKeys.get(tag);
    if (this.liveTagKey.has(tag) && sameKey(this.liveTagKey.get(tag) as K, key)) {
      if (!extraKeys) {
        this.liveTagKey.delete(tag);
        return;
      }
      // Another key still holds the tag: it takes over the main slot.
      const [heir] = extraKeys;
      this.liveTagKey.set(tag, heir);
      extraKeys.delete(heir);
    } else {
      extraKeys?.delete(key);
    }
    if (extraKeys && extraKeys.size === 0) this.liveTagExtraKeys.delete(tag);
  }

  /**
   * A copy of the live-tag index: every tag that has a record, with the keys
   * holding one. Exists so tests can check the index against `items`.
   *
   * @internal
   */
  public getLiveTagIndex(): Map<string, K[]> {
    const index = new Map<string, K[]>();
    for (const [tag, key] of this.liveTagKey) {
      index.set(tag, [key]);
    }
    for (const [tag, extraKeys] of this.liveTagExtraKeys) {
      const holders = index.get(tag) ?? [];
      holders.push(...extraKeys);
      index.set(tag, holders);
    }
    return index;
  }

  /**
   * Merges state from another ORMap.
   * - Adds all new tombstones from 'other', and its per-key attribution.
   * - Adds all new items from 'other' that are not in tombstones.
   * - Updates HLC with observed timestamps.
   */
  public merge(other: ORMap<K, V>): void {
    const changedKeys = new Set<K>();

    // 1. Merge tombstones
    for (const tag of other.tombstones) {
      this.tombstones.add(tag);
    }

    // Attribution is a per-key union. Every attributed tag of `other` is in
    // its map-wide set, which was just merged, so the union cannot attribute
    // a tag this map does not suppress.
    for (const [key, otherAttributed] of other.keyTombstones) {
      const before = this.keyTombstones.get(key)?.size ?? 0;
      for (const tag of otherAttributed) {
        this.attribute(key, tag);
      }
      if ((this.keyTombstones.get(key)?.size ?? 0) !== before) {
        changedKeys.add(key);
      }
    }

    // 2. Merge items
    for (const [key, otherKeyMap] of other.items) {
      const localKeyMap = this.keyMapFor(key);

      for (const [tag, record] of otherKeyMap) {
        // Only accept if not deleted
        if (!this.tombstones.has(tag)) {
          if (!localKeyMap.has(tag)) {
            this.putRecord(key, localKeyMap, record);
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
          this.dropRecord(key, localKeyMap, tag);
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
      } catch {
        // Ignore invalid tags
      }
    }

    // A pruned tag must leave every key's attribution as well: the server
    // drops it from the key's leaf when it prunes, and an attributed tag that
    // is no longer suppressed map-wide would break "attributed implies
    // tombstoned".
    if (removedTags.length > 0) {
      for (const [key, attributed] of this.keyTombstones) {
        let changed = false;
        for (const tag of removedTags) {
          if (attributed.delete(tag)) changed = true;
        }
        if (!changed) continue;
        if (attributed.size === 0) this.keyTombstones.delete(key);
        this.updateMerkleTree(key);
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
      tombstones: this.tombstones,
      keyTombstones: this.keyTombstones,
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
    remoteTombstones: string[] = [],
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
    const localKeyMap = this.keyMapFor(key);

    // Remove any local records that are now tombstoned
    for (const tag of localKeyMap.keys()) {
      if (this.tombstones.has(tag)) {
        this.dropRecord(key, localKeyMap, tag);
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
        this.putRecord(key, localKeyMap, remoteRecord);
        added++;
      } else if (compareTimestamps(remoteRecord.timestamp, localRecord.timestamp) > 0) {
        // Remote is newer - update
        this.putRecord(key, localKeyMap, remoteRecord);
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
   *
   * The leaf covers every tag held under the key, including records whose TTL
   * has passed (the server keeps those until they are removed), plus the
   * tombstones attributed to the key. The tree drops the key when both are
   * empty, so a key holding only tombstones stays in it.
   */
  private updateMerkleTree(key: K): void {
    const noTags: string[] = [];
    this.merkleTree.update(
      String(key),
      this.items.get(key)?.keys() ?? noTags,
      this.keyTombstones.get(key) ?? noTags,
    );
  }
}
