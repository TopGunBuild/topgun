import { hashString } from '@topgunbuild/core';
import type { ORMap } from '@topgunbuild/core';
import type { IStorageAdapter } from '../IStorageAdapter';
import { logger } from './logger';

/**
 * Persisted form of one bucket of an OR-Map's per-key tombstone attribution:
 * `[key, tags[]]` pairs rather than an object keyed by map key. A map key is
 * arbitrary user data, and one named `__proto__` would not survive as an own
 * property of a plain object.
 */
export type PersistedKeyTombstones = Array<[string, string[]]>;

/**
 * The attribution is stored in up to 256 meta entries per map rather than one.
 * A single entry holding every key has to be rewritten whole on each local
 * remove and each server-reported change, and a map whose keys share a large
 * tombstone set makes that value tens of megabytes; a bucket holds only the
 * keys that share the first two hex characters of their Merkle path, so one
 * write costs about 1/256 of the map.
 *
 * Bucketing by Merkle path rather than, say, by insertion order is what makes
 * a sync response cheap to persist: a leaf response covers the keys of one
 * Merkle path, so everything it changes lands in one bucket.
 */
const BUCKET_ID_LENGTH = 2;
const BUCKET_ID_RE = /^[0-9a-f]{2}$/;

const bucketKeyPrefix = (mapName: string): string => `__sys__:${mapName}:keyTombstones:`;

/**
 * The bucket a key's attribution is stored in: the first two hex characters of
 * the key's path in the OR-Map Merkle tree.
 *
 * The path is derived exactly as `ORMapMerkleTree` derives it (the tree keeps
 * that derivation private, so it is repeated here over the same exported
 * `hashString`). If the two ever disagreed, nothing would be lost or mismatched:
 * a bucket is only a storage partition, every existing bucket is read back on
 * load, and all that would change is that one leaf response could touch more
 * than one bucket.
 */
export function orMapKeyTombstonesBucketOf(key: string): string {
  return hashString(key).toString(16).padStart(8, '0').slice(0, BUCKET_ID_LENGTH);
}

/**
 * Meta key of one attribution bucket of a map. It ends in the bucket id, never
 * in `:ormap` or `:tombstones`, so the held-map enumeration cannot match it
 * whatever the map is called: a bucket is always written alongside the `:ormap`
 * existence marker and never needs to announce a map on its own, and matching
 * it would yield a bogus map name.
 */
export const orMapKeyTombstonesBucketKey = (mapName: string, bucket: string): string =>
  `${bucketKeyPrefix(mapName)}${bucket}`;

/**
 * The attribution bucket keys of `mapName` among `metaKeys`.
 *
 * Only a key that is the map's prefix followed by exactly one bucket id counts.
 * That keeps two maps apart when one name extends the other with a colon
 * (`m` and `m:keyTombstones`): the longer map's bucket keys continue with more
 * than a bucket id after the shorter map's prefix. It also leaves out the
 * un-bucketed `__sys__:{mapName}:keyTombstones` entry an unreleased build
 * wrote: nothing reads that entry any more.
 */
export function orMapKeyTombstonesBucketKeys(
  mapName: string,
  metaKeys: Iterable<string>,
): string[] {
  const prefix = bucketKeyPrefix(mapName);
  const bucketKeys: string[] = [];
  for (const metaKey of metaKeys) {
    if (metaKey.startsWith(prefix) && BUCKET_ID_RE.test(metaKey.slice(prefix.length))) {
      bucketKeys.push(metaKey);
    }
  }
  return bucketKeys;
}

/**
 * The current attribution of every key of `map` that belongs to `bucket`, in
 * its persisted form; empty when the bucket holds no attributed key.
 *
 * Every writer stores a bucket's full value (never a per-key patch), so the
 * local-write commit and the server-origin persist cannot leave an entry in two
 * different shapes. Build the value immediately before the storage call that
 * writes it, with no `await` in between: a value built earlier can be
 * overtaken by another writer of the same bucket and would then overwrite the
 * newer one.
 *
 * `removed` names a key of this bucket and the tags a local remove took from
 * it, for the commit that makes that remove durable. They are listed under the
 * key whether or not the map still attributes them to it. The commit can wait,
 * and a server response handled during the wait finds no pending remove in the
 * op log yet, so it may replace the key's attribution with a set that lacks
 * these tags; the bucket written with the op must still carry them, or a
 * reload would hold the remove without its attribution.
 */
export function serializeOrMapKeyTombstonesBucket(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the persisted form only needs the key as a string and the tags; the map's value type is irrelevant here
  map: ORMap<any, any>,
  bucket: string,
  removed?: { key: string; tags: readonly string[] },
): PersistedKeyTombstones {
  const pairs: PersistedKeyTombstones = [];
  let removedPair: [string, string[]] | undefined;
  for (const [key, tags] of map.getSnapshot().keyTombstones) {
    const name = String(key);
    if (orMapKeyTombstonesBucketOf(name) === bucket) {
      const pair: [string, string[]] = [name, Array.from(tags)];
      if (removed !== undefined && name === removed.key) removedPair = pair;
      pairs.push(pair);
    }
  }
  if (removed !== undefined && removed.tags.length > 0) {
    if (removedPair === undefined) {
      pairs.push([removed.key, Array.from(removed.tags)]);
    } else {
      const listed = new Set(removedPair[1]);
      for (const tag of removed.tags) {
        if (!listed.has(tag)) removedPair[1].push(tag);
      }
    }
  }
  return pairs;
}

/**
 * Tracks, per map, the periods in which an attribution bucket must not be
 * written from the map in memory, because memory does not describe what the
 * bucket should hold:
 *
 * - While the map is loading. Until the persisted attribution has been applied,
 *   the map holds only what this session added, and a bucket written from it
 *   would replace the persisted one with that fragment.
 * - While the map is being reset. The reset empties the buckets one write at a
 *   time and only then clears the map, so a bucket written in between would be
 *   refilled from attribution that is about to be discarded, and stay on disk
 *   after the clear.
 *
 * A writer that can wait does so (`whenReleased`) and writes afterwards. One
 * that cannot, because its value must be built synchronously for a commit,
 * leaves the bucket out and records it (`defer`); whoever ends the last hold
 * gets those buckets back and decides whether they are still worth writing.
 *
 * Holds on one map can overlap (a reset can start while the map is loading), so
 * they are counted.
 */
export class OrMapKeyTombstonesWriteHolds {
  private readonly holds = new Map<
    string,
    { count: number; deferred: Set<string>; released: Promise<void>; release: () => void }
  >();

  /** Starts a hold on `mapName`. Every call must be matched by one `end`. */
  begin(mapName: string): void {
    const hold = this.holds.get(mapName);
    if (hold) {
      hold.count++;
      return;
    }
    let release: () => void = () => undefined;
    const released = new Promise<void>((resolve) => {
      release = resolve;
    });
    this.holds.set(mapName, { count: 1, deferred: new Set(), released, release });
  }

  isHeld(mapName: string): boolean {
    return this.holds.has(mapName);
  }

  /** Records a bucket of a held map that a writer left out. */
  defer(mapName: string, bucket: string): void {
    this.holds.get(mapName)?.deferred.add(bucket);
  }

  /**
   * Resolves when the hold that is on `mapName` now has ended. A new hold can
   * begin before the waiter runs again, so a writer re-checks `isHeld` after
   * the wait and builds its value only once that check passes, with no `await`
   * in between.
   */
  whenReleased(mapName: string): Promise<void> {
    return this.holds.get(mapName)?.released ?? Promise.resolve();
  }

  /**
   * Ends one hold on `mapName`.
   *
   * `keepDeferred` false discards the buckets deferred so far: the holder knows
   * that writing them from memory would be wrong or pointless.
   *
   * @returns the deferred buckets to write now, once the last hold has ended;
   * empty while another hold is still on the map
   */
  end(mapName: string, keepDeferred: boolean): string[] {
    const hold = this.holds.get(mapName);
    if (!hold) return [];
    if (!keepDeferred) hold.deferred.clear();
    if (--hold.count > 0) return [];
    this.holds.delete(mapName);
    hold.release();
    return Array.from(hold.deferred);
  }
}

/** The part of a storage adapter that loading and resetting the attribution need. */
type AttributionStorage = Pick<IStorageAdapter, 'getAllMetaKeys' | 'getMeta' | 'setMeta'>;

/**
 * Loads a map's persisted attribution from storage and re-applies it. Shared by
 * both OR-Map restore seams (`TopGunClient.restoreORMap` and
 * `SyncEngine.instantiateAndRestoreOrMap`) so they cannot diverge.
 *
 * The meta keys are listed once to find the buckets that exist (the adapter has
 * no way to read several entries by prefix), and all buckets are then applied
 * together.
 *
 * A failed listing rejects: nothing is known about the buckets then. A failed
 * read of one bucket (a storage or decryption error) does not: that bucket is
 * reported and left out, and the others are applied. What is lost with it is
 * what `restoreOrMapKeyTombstones` describes for unreadable content, and it is
 * confined to the keys of that bucket instead of costing the map all of its
 * attribution.
 */
export async function loadOrMapKeyTombstones(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- restore runs at the registry level where the map's key and value types are erased
  map: ORMap<any, any>,
  mapName: string,
  storage: AttributionStorage,
): Promise<void> {
  const bucketKeys = orMapKeyTombstonesBucketKeys(mapName, await storage.getAllMetaKeys());
  if (bucketKeys.length === 0) return;
  const reads = await Promise.allSettled(bucketKeys.map((bucketKey) => storage.getMeta(bucketKey)));
  const buckets: unknown[] = [];
  for (const [i, read] of reads.entries()) {
    if (read.status === 'fulfilled') {
      buckets.push(read.value);
    } else {
      logger.warn(
        { mapName, bucketKey: bucketKeys[i], err: read.reason },
        'Could not read a persisted OR-Map tombstone attribution bucket; its tags are not attributed, and those listed only there are not suppressed, until the next sync walk rewrites it',
      );
    }
  }
  restoreOrMapKeyTombstones(map, buckets);
}

/**
 * Durably empties every attribution bucket the store holds for `mapName`.
 *
 * The buckets are found by listing the store, not derived from the map in
 * memory: storage can hold a bucket the map no longer has a key for, and one
 * left non-empty would come back on the next load as tombstones the server
 * does not attribute to anything.
 *
 * Call it BEFORE the in-memory map is cleared, and with attribution writes of
 * the map held (`OrMapKeyTombstonesWriteHolds`) until the clear has run: the
 * buckets are emptied over many awaited writes, and a bucket written from the
 * not-yet-cleared map in between would be refilled and outlive the clear. A
 * rejection propagates, and the caller must then leave the map as it is. Buckets are written one at a time,
 * so a failure part-way leaves some of them empty on disk while memory still
 * holds the full attribution. That is the safe direction: a reload sees less
 * attribution than the server has, the Merkle roots differ, and one sync walk
 * restores it. The opposite (memory cleared, disk still attributing) would
 * re-attribute tags after a reload that the wiped map no longer accounts for.
 *
 * Entries are emptied, not deleted: the adapter can delete a meta entry only
 * as part of an op commit.
 */
export async function resetPersistedOrMapKeyTombstones(
  mapName: string,
  storage: AttributionStorage,
): Promise<void> {
  const bucketKeys = orMapKeyTombstonesBucketKeys(mapName, await storage.getAllMetaKeys());
  for (const bucketKey of bucketKeys) {
    await storage.setMeta(bucketKey, []);
  }
}

/**
 * Re-applies a persisted attribution, given as the values of its buckets, to a
 * map being loaded from storage.
 *
 * Call it AFTER the map's records are loaded. Storage can hold a record whose
 * tag is listed only here: a server-reported attribution purges the record in
 * memory and persists this entry, but re-persists the records of the reported
 * key only, not of another key the record happened to live under. Attributing
 * the tag again on load purges that record again and puts the tag back into the
 * map-wide tombstone set, so "attributed implies tombstoned" holds after a
 * reload.
 *
 * Never throws, and skips whatever it cannot read: a bucket that is not a list,
 * a pair that is not `[key, tags[]]`, a tag that is not a string. A store
 * written before attribution existed has no bucket at all and loads with none.
 *
 * Skipping a malformed entry is not free, and it loses two different things:
 *
 * - The key's Merkle leaf no longer matches the server's. That alone costs one
 *   sync walk, which mirrors the server's set and rewrites the entry. It can
 *   never make two different states compare equal.
 * - For some tags, the map-wide suppression itself. The `:tombstones` entry is
 *   rewritten only when a server response adds or updates a record, so a tag
 *   learned from a response that changed no record is durable in this entry and
 *   nowhere else. Skip it and the tag is not suppressed after the load: a
 *   record carrying it that storage still holds under another key is visible
 *   again, and one that arrives later is admitted, until a sync walk reports
 *   the tag once more. The leaf mismatch above is what triggers that walk, so
 *   the window closes on the next successful sync; a client that stays offline
 *   stays in it.
 *
 * Skipping is still the right call. The alternative is refusing to load the
 * map, which trades a value that may be shown once too often for every value
 * of the map being unavailable, offline included; and an entry that cannot be
 * read cannot be repaired locally either way: only the server can say which
 * tags it held. The loss is reported in the log rather than hidden.
 *
 * The persisted tags are merged into whatever the map already attributes to the
 * key rather than replacing it. The load is asynchronous, and a remove (local
 * or from the server) that lands on the map while it is still loading has
 * already attributed its tag; a replace would silently drop it.
 *
 * All readable pairs of all buckets are attributed in ONE pass over the map:
 * the live records are purged once for every persisted tag together and each
 * key's leaf is hashed once. Attributing key by key would hash and notify per key, and the
 * load would block its thread for as long as that takes.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- restore runs at the registry level where the map's key and value types are erased; persisted keys are strings
export function restoreOrMapKeyTombstones(map: ORMap<any, any>, buckets: Iterable<unknown>): void {
  let skipped = 0;
  const readable: Array<[string, string[]]> = [];
  for (const persisted of buckets) {
    // Absent is ordinary (a bucket nothing was ever written to), not damage.
    if (persisted === undefined || persisted === null) continue;
    if (!Array.isArray(persisted)) {
      skipped++;
      continue;
    }
    for (const pair of persisted) {
      if (!Array.isArray(pair)) {
        skipped++;
        continue;
      }
      const [key, tags] = pair as unknown[];
      if (typeof key !== 'string' || !Array.isArray(tags)) {
        skipped++;
        continue;
      }

      const readableTags = stringsOf(tags);
      skipped += tags.length - readableTags.length;
      if (readableTags.length > 0) readable.push([key, readableTags]);
    }
  }

  if (readable.length > 0) map.addKeyTombstones(readable);

  // Skipping must not be silent: unreadable attribution means durable state
  // was damaged, and for some tags it held the only durable copy of their
  // suppression.
  if (skipped > 0) {
    logger.warn(
      { skipped },
      'Skipped unreadable persisted OR-Map tombstone attribution; tags listed only there are not suppressed until the next sync walk rewrites it',
    );
  }
}

/**
 * The string elements of `values`. Returns `values` itself when every element
 * is a string, which is the ordinary case: a stored attribution can hold
 * millions of tags, and copying them all just to validate them would double the
 * memory a load needs.
 */
function stringsOf(values: unknown[]): string[] {
  let allStrings = true;
  // Indexed on purpose: a hole in a sparse array reads as `undefined` here,
  // while the array's own iteration helpers would step over it unchecked.
  for (let i = 0; i < values.length; i++) {
    if (typeof values[i] !== 'string') {
      allStrings = false;
      break;
    }
  }
  if (allStrings) return values as string[];

  const strings: string[] = [];
  for (let i = 0; i < values.length; i++) {
    const value = values[i];
    if (typeof value === 'string') strings.push(value);
  }
  return strings;
}
