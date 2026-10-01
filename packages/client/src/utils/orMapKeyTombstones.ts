import type { ORMap } from '@topgunbuild/core';

/**
 * Persisted form of an OR-Map's per-key tombstone attribution: `[key, tags[]]`
 * pairs rather than an object keyed by map key. A map key is arbitrary user
 * data, and one named `__proto__` would not survive as an own property of a
 * plain object.
 */
export type PersistedKeyTombstones = Array<[string, string[]]>;

/**
 * Meta key holding an OR-Map's per-key tombstone attribution. The suffix is
 * deliberately one the held-map enumeration does not match: this key is always
 * written alongside the `:ormap` existence marker, so it never needs to
 * announce a map on its own, and matching it would yield a bogus map name.
 */
export const orMapKeyTombstonesKey = (mapName: string): string =>
  `__sys__:${mapName}:keyTombstones`;

/**
 * The map's whole current attribution in its persisted form. Every writer
 * stores this full value (never a per-key patch), so the local-write commit and
 * the server-origin persist cannot leave the entry in two different shapes.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the persisted form only needs the key as a string and the tags; the map's value type is irrelevant here
export function serializeOrMapKeyTombstones(map: ORMap<any, any>): PersistedKeyTombstones {
  const pairs: PersistedKeyTombstones = [];
  for (const [key, tags] of map.getSnapshot().keyTombstones) {
    pairs.push([String(key), Array.from(tags)]);
  }
  return pairs;
}

/**
 * Re-applies a persisted attribution to a map being loaded from storage. Shared
 * by both OR-Map restore seams (`TopGunClient.restoreORMap` and
 * `SyncEngine.instantiateAndRestoreOrMap`) so they cannot diverge.
 *
 * Call it AFTER the map's records are loaded. Storage can hold a record whose
 * tag is listed only here: a server-reported attribution purges the record in
 * memory and persists this entry, but re-persists the records of the reported
 * key only, not of another key the record happened to live under. Attributing
 * the tag again on load purges that record again and puts the tag back into the
 * map-wide tombstone set, so "attributed implies tombstoned" holds after a
 * reload.
 *
 * Never throws, and skips whatever it cannot read. A store written before
 * attribution existed has no entry at all and loads with none. Skipping is the
 * safe direction for a malformed entry too: a missing attribution only makes
 * this client's Merkle root differ from the server's, which costs one sync walk
 * that rewrites the entry — it can never make two different states compare
 * equal. Refusing to load the map over it would be the worse outcome.
 *
 * The persisted tags are merged into whatever the map already attributes to the
 * key rather than replacing it. The load is asynchronous, and a remove (local
 * or from the server) that lands on the map while it is still loading has
 * already attributed its tag; a replace would silently drop it.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- restore runs at the registry level where the map's key and value types are erased; persisted keys are strings
export function restoreOrMapKeyTombstones(map: ORMap<any, any>, persisted: unknown): void {
  if (!Array.isArray(persisted)) return;

  for (const pair of persisted) {
    if (!Array.isArray(pair)) continue;
    const [key, tags] = pair as unknown[];
    if (typeof key !== 'string' || !Array.isArray(tags)) continue;

    const merged = map.getKeyTombstones(key);
    for (const tag of tags) {
      if (typeof tag === 'string') merged.add(tag);
    }
    if (merged.size === 0) continue;
    map.setKeyTombstones(key, merged);
  }
}
