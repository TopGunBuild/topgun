import { HLC, ORMap } from '@topgunbuild/core';
import type { ORMapRecord } from '@topgunbuild/core';
import { restoreOrMapKeyTombstones } from '../utils/orMapKeyTombstones';
import type { PersistedKeyTombstones } from '../utils/orMapKeyTombstones';

/**
 * Cost bounds for re-applying a persisted per-key tombstone attribution.
 *
 * The restore runs synchronously on every load of an OR-Map, after its records
 * are in memory, and nothing is attributed yet at that point: every persisted
 * tag is new to its key. If each key's restore walks the whole map, opening a
 * large map blocks the thread for minutes.
 *
 * These are regression guards, not benchmarks. Each bound sits about an order
 * of magnitude above what attributing the same tags costs on a map that holds
 * no live record at all (which is all the work that is left once nothing is
 * scanned), so it only trips when the cost grows with keys x tags again. The
 * Jest timeout is far above the bound on purpose: a regression has to fail on
 * the elapsed-time assertion, with the measured number in the message, not on a
 * runner timeout that says nothing.
 */

/** Jest timeout for a bounded section; never the reason a run fails. */
const JEST_TIMEOUT_MS = 600_000;

/**
 * 3 000 keys that each hold the same 3 000 tags: the shape left behind by a
 * client that used to push its map-wide tombstone set with every key.
 * Attributing those tags on a map with no live record takes about 3 s on a
 * 2021 laptop, nearly all of it hashing a 3 000-tag leaf per key, so that is
 * the floor a restore of this shape can reach without changing the leaf hash.
 */
const SHARED_TAGS_RESTORE_BOUND_MS = 30_000;

/**
 * 20 000 keys with 5 tags of their own each. Attributing those tags on a map
 * with no live record takes about 0.13 s on a 2021 laptop.
 */
const PER_KEY_RESTORE_BOUND_MS = 2_000;

const keyOf = (i: number): string => `key-${i}`;

function mapWithOneLiveRecordPerKey(keys: number): ORMap<string, string> {
  const map = new ORMap<string, string>(new HLC('n1'));
  for (let i = 0; i < keys; i++) {
    map.add(keyOf(i), 'live');
  }
  return map;
}

describe('restoreOrMapKeyTombstones cost', () => {
  test(
    'restoring 3 000 keys that share 3 000 tombstones stays within the bound',
    () => {
      const KEYS = 3_000;
      const SHARED_TAGS = 3_000;
      const map = mapWithOneLiveRecordPerKey(KEYS);
      const sharedTags = Array.from({ length: SHARED_TAGS }, (_, j) => `1700000000000:${j}:gone`);
      const persisted: PersistedKeyTombstones = Array.from({ length: KEYS }, (_, i) => [
        keyOf(i),
        sharedTags,
      ]);

      // Storage can hold a record whose tag is listed only in the persisted
      // attribution. The restore has to remove it, so a version that never
      // looks at the records cannot pass.
      const doomed: ORMapRecord<string> = {
        value: 'doomed',
        tag: sharedTags[SHARED_TAGS - 1],
        timestamp: { millis: 1_700_000_000_000, counter: SHARED_TAGS - 1, nodeId: 'gone' },
      };
      expect(map.apply('holder-of-a-removed-record', doomed)).toBe(true);

      const start = performance.now();
      restoreOrMapKeyTombstones(map, persisted);
      const elapsedMs = performance.now() - start;

      expect(map.get('holder-of-a-removed-record')).toEqual([]);
      expect(map.size).toBe(KEYS);
      expect(map.totalRecords).toBe(KEYS);
      expect(map.getTombstones()).toHaveLength(SHARED_TAGS);
      const attribution = map.getSnapshot().keyTombstones;
      expect(attribution.size).toBe(KEYS);
      for (let i = 0; i < KEYS; i++) {
        const attributed = attribution.get(keyOf(i));
        if (
          !attributed ||
          attributed.size !== SHARED_TAGS ||
          !sharedTags.every((tag) => attributed.has(tag))
        ) {
          throw new Error(`key ${keyOf(i)} was not restored with exactly the shared tags`);
        }
      }
      expect(map.get(keyOf(0))).toEqual(['live']);
      expect(map.get(keyOf(KEYS - 1))).toEqual(['live']);

      // Asserted last, so a run that is too slow still proves the result above.
      expect(elapsedMs).toBeLessThanOrEqual(SHARED_TAGS_RESTORE_BOUND_MS);
    },
    JEST_TIMEOUT_MS,
  );

  test(
    'restoring 20 000 keys with 5 tombstones each stays within the bound',
    () => {
      const KEYS = 20_000;
      const TAGS_PER_KEY = 5;
      const map = mapWithOneLiveRecordPerKey(KEYS);
      const persisted: PersistedKeyTombstones = Array.from({ length: KEYS }, (_, i) => [
        keyOf(i),
        Array.from({ length: TAGS_PER_KEY }, (_, j) => `1700000000000:${j}:gone-${i}`),
      ]);

      const start = performance.now();
      restoreOrMapKeyTombstones(map, persisted);
      const elapsedMs = performance.now() - start;

      expect(map.size).toBe(KEYS);
      expect(map.totalRecords).toBe(KEYS);
      expect(map.getTombstones()).toHaveLength(KEYS * TAGS_PER_KEY);
      const attribution = map.getSnapshot().keyTombstones;
      expect(attribution.size).toBe(KEYS);
      for (const [key, tags] of persisted) {
        const attributed = attribution.get(key);
        if (
          !attributed ||
          attributed.size !== TAGS_PER_KEY ||
          !tags.every((tag) => attributed.has(tag))
        ) {
          throw new Error(`key ${key} was not restored with exactly its persisted tags`);
        }
      }
      expect(map.get(keyOf(0))).toEqual(['live']);
      expect(map.get(keyOf(KEYS - 1))).toEqual(['live']);

      // Asserted last, so a run that is too slow still proves the result above.
      expect(elapsedMs).toBeLessThanOrEqual(PER_KEY_RESTORE_BOUND_MS);
    },
    JEST_TIMEOUT_MS,
  );
});
