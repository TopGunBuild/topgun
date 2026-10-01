import type { ORMap } from '@topgunbuild/core';
import { logger } from './logger';

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
 * attribution existed has no entry at all and loads with none.
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
 * All readable pairs are attributed in ONE pass over the map: the live records
 * are purged once for every persisted tag together and each key's leaf is
 * hashed once. Attributing key by key would hash and notify per key, and the
 * load would block its thread for as long as that takes.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- restore runs at the registry level where the map's key and value types are erased; persisted keys are strings
export function restoreOrMapKeyTombstones(map: ORMap<any, any>, persisted: unknown): void {
  // Absent is the ordinary case (a store written before attribution existed).
  if (persisted === undefined || persisted === null) return;

  let skipped = 0;
  const readable: Array<[string, string[]]> = [];
  if (!Array.isArray(persisted)) {
    skipped = 1;
  } else {
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
