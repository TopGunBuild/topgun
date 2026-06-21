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
 * Structural equality for record values, used to decide whether a rejected
 * server echo is a pure re-stamp of the same data (safe to adopt) or a
 * genuinely different (newer local) write that must be preserved. Order-stable
 * over object keys so two structurally-equal values that survived a serialize/
 * deserialize round-trip compare equal regardless of key insertion order.
 */
function valuesEqual(a: unknown, b: unknown, depth = 0): boolean {
  if (a === b) return true;
  if (a === null || b === null || a === undefined || b === undefined) {
    return a === b;
  }
  if (typeof a !== 'object' || typeof b !== 'object') return false;

  // Depth guard: record values reach this only via the server-echo reconcile
  // path and normally round-trip through msgpack (which rejects cycles), but a
  // pathological or cyclic in-memory value must not stack-overflow the sync
  // handler. Treat over-deep structures as "not equal" → conservatively skip
  // adoption rather than crash.
  if (depth > 64) return false;

  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((item, i) => valuesEqual(item, b[i], depth + 1));
  }

  const aObj = a as Record<string, unknown>;
  const bObj = b as Record<string, unknown>;
  const aKeys = Object.keys(aObj);
  const bKeys = Object.keys(bObj);
  if (aKeys.length !== bKeys.length) return false;
  const hasOwn = Object.prototype.hasOwnProperty;
  return aKeys.every((k) => hasOwn.call(bObj, k) && valuesEqual(aObj[k], bObj[k], depth + 1));
}

/**
 * Last-Write-Wins Map Implementation.
 * This structure guarantees convergence by always keeping the entry with the highest timestamp.
 */
export class LWWMap<K, V> {
  private data: Map<K, LWWRecord<V>>;
  private readonly hlc: HLC;
  private listeners: Array<(entries: Array<[K, V]>) => void> = [];
  private merkleTree: MerkleTree;

  constructor(hlc: HLC) {
    this.hlc = hlc;
    this.data = new Map();
    this.merkleTree = new MerkleTree();
  }

  public subscribe(callback: (entries: Array<[K, V]>) => void): () => void {
    this.listeners.push(callback);
    return () => {
      this.listeners = this.listeners.filter((cb) => cb !== callback);
    };
  }

  private notify(): void {
    if (this.listeners.length === 0) return;
    const snapshot = Array.from(this.entries()) as Array<[K, V]>;
    this.listeners.forEach((cb) => cb(snapshot));
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

    // Check for expiration using HLC's clock source
    if (record.ttlMs) {
      const now = this.hlc.getClockSource().now();
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
   * Reconciles the server's authoritative re-stamp of one of this client's own
   * optimistic writes.
   *
   * Optimistic writes are stamped with the client's HLC; the server re-stamps
   * with its own arrival-order HLC and echoes the record back. When the client
   * clock briefly outran the server, the echo's timestamp is ≤ the local one,
   * so {@link merge} rejects it and memory keeps the client timestamp. Because
   * the Merkle tree hashes only `key:timestamp` (not the value), that leaves the
   * client's Merkle root permanently diverged from the server's — the same value
   * under two different timestamps — defeating the "root-equal ⇒ in-sync" fast
   * path and forcing endless bucket re-requests until a reload re-hydrates from
   * disk.
   *
   * Adopting the server's record aligns memory, the Merkle tree, and (via the
   * caller's persistence) disk with the server. This is only safe when the local
   * value is unchanged: if it differs, the client holds a strictly newer write
   * the server has not seen yet, which must be preserved (no data loss).
   *
   * Call this only after {@link merge} has returned `false` for a server echo.
   *
   * @returns `true` if the server record was adopted (the caller should persist
   *   it so disk matches memory); `false` if a newer local write supersedes the
   *   echo (the caller must NOT persist the stale server record).
   */
  public adoptServerEcho(key: K, serverRecord: LWWRecord<V>): boolean {
    const localRecord = this.data.get(key);
    if (!localRecord || !valuesEqual(localRecord.value, serverRecord.value)) {
      return false;
    }
    // Defensive precondition: this path only reconciles an echo that LWW
    // rejected because the server timestamp is ≤ ours. If the server record is
    // strictly newer, plain `merge` already wins it — never downgrade a newer
    // record through here (guards against misuse if merge semantics change).
    if (HLC.compare(serverRecord.timestamp, localRecord.timestamp) > 0) {
      return false;
    }
    this.data.set(key, serverRecord);
    this.merkleTree.update(String(key), serverRecord);
    this.notify();
    return true;
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
    const clockSource = this.hlc.getClockSource();

    return {
      [Symbol.iterator]() {
        return this;
      },
      next: () => {
        let result = iterator.next();
        while (!result.done) {
          const [key, record] = result.value;
          if (record.value !== null) {
            // Check TTL using clock source
            if (record.ttlMs && record.timestamp.millis + record.ttlMs < clockSource.now()) {
              result = iterator.next();
              continue;
            }
            return { value: [key, record.value], done: false };
          }
          result = iterator.next();
        }
        return { value: undefined, done: true };
      },
    };
  }

  /**
   * Returns all keys (including tombstones).
   */
  public allKeys(): IterableIterator<K> {
    return this.data.keys();
  }
}
