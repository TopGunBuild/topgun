import { HLC } from '../HLC';
import { ORMap } from '../ORMap';
import type { ORMapRecord } from '../ORMap';

/**
 * Cost bounds for per-key tombstone attribution.
 *
 * A tombstone tag is normally live nowhere: the record it names was removed
 * when the tag was created. Attributing such a tag to a key must therefore not
 * cost a walk over every key in the map, or a sync walk (one attribution per
 * visited key) and a load from storage (one per persisted key) both turn
 * quadratic in the size of the map and freeze the thread they run on.
 *
 * These are regression guards, not benchmarks. Each bound sits about an order
 * of magnitude above what the same calls cost on a map that holds no live
 * record at all (which is all the work that is left once nothing is scanned),
 * so it only trips when the cost grows with keys x tags again. The Jest
 * timeout is far above the bound on purpose: a regression has to fail on the
 * elapsed-time assertion, with the measured number in the message, not on a
 * runner timeout that says nothing.
 */

/** Jest timeout for a bounded section; never the reason a run fails. */
const JEST_TIMEOUT_MS = 600_000;

/**
 * 20 000 keys, 5 tags each, one attribution call per key. The same calls on a
 * map with no live record take about 0.13 s on a 2021 laptop.
 */
const PER_KEY_WALK_BOUND_MS = 2_000;

/**
 * 1 000 keys that each receive the same 1 000 tags. The same calls on a map
 * with no live record take about 0.3 s on a 2021 laptop; nearly all of it is
 * hashing a 1 000-tag leaf per key.
 */
const SHARED_TAGS_BOUND_MS = 5_000;

const keyOf = (i: number): string => `key-${i}`;

function mapWithOneLiveRecordPerKey(keys: number): ORMap<string, string> {
  const map = new ORMap<string, string>(new HLC('n1'));
  for (let i = 0; i < keys; i++) {
    map.add(keyOf(i), 'live');
  }
  return map;
}

describe('ORMap per-key tombstone attribution cost', () => {
  test(
    'attributing 5 tags that are live nowhere to each of 20 000 keys stays within the bound',
    () => {
      const KEYS = 20_000;
      const TAGS_PER_KEY = 5;
      const map = mapWithOneLiveRecordPerKey(KEYS);
      const tagsOf = (i: number): string[] =>
        Array.from({ length: TAGS_PER_KEY }, (_, j) => `1700000000000:${j}:gone-${i}`);
      const tags = Array.from({ length: KEYS }, (_, i) => tagsOf(i));

      const start = performance.now();
      for (let i = 0; i < KEYS; i++) {
        map.setKeyTombstones(keyOf(i), tags[i]);
      }
      const elapsedMs = performance.now() - start;

      // The bound must not be met by skipping the work: every key holds its
      // attribution, every tag is suppressed map-wide, and no live record moved.
      expect(map.size).toBe(KEYS);
      expect(map.totalRecords).toBe(KEYS);
      expect(map.getTombstones()).toHaveLength(KEYS * TAGS_PER_KEY);
      for (let i = 0; i < KEYS; i++) {
        const attributed = map.getKeyTombstones(keyOf(i));
        if (attributed.size !== TAGS_PER_KEY || !tags[i].every((tag) => attributed.has(tag))) {
          throw new Error(`key ${keyOf(i)} does not hold exactly the tags attributed to it`);
        }
      }
      expect(map.get(keyOf(0))).toEqual(['live']);
      expect(map.get(keyOf(KEYS - 1))).toEqual(['live']);

      // Asserted last, so a run that is too slow still proves the result above.
      expect(elapsedMs).toBeLessThanOrEqual(PER_KEY_WALK_BOUND_MS);
    },
    JEST_TIMEOUT_MS,
  );

  test(
    'attributing the same 1 000 tags to each of 1 000 keys stays within the bound',
    () => {
      const KEYS = 1_000;
      const SHARED_TAGS = 1_000;
      const map = mapWithOneLiveRecordPerKey(KEYS);
      const sharedTags = Array.from({ length: SHARED_TAGS }, (_, j) => `1700000000000:${j}:gone`);

      // One live record does carry an attributed tag, under a key of its own.
      // Attribution has to remove it, so a version that never looks at the
      // records cannot pass.
      const doomed: ORMapRecord<string> = {
        value: 'doomed',
        tag: sharedTags[SHARED_TAGS - 1],
        timestamp: { millis: 1_700_000_000_000, counter: SHARED_TAGS - 1, nodeId: 'gone' },
      };
      expect(map.apply('holder-of-a-removed-record', doomed)).toBe(true);
      expect(map.get('holder-of-a-removed-record')).toEqual(['doomed']);

      const start = performance.now();
      for (let i = 0; i < KEYS; i++) {
        map.setKeyTombstones(keyOf(i), sharedTags);
      }
      const elapsedMs = performance.now() - start;

      expect(map.get('holder-of-a-removed-record')).toEqual([]);
      expect(map.size).toBe(KEYS);
      expect(map.totalRecords).toBe(KEYS);
      expect(map.getTombstones()).toHaveLength(SHARED_TAGS);
      for (let i = 0; i < KEYS; i++) {
        const attributed = map.getKeyTombstones(keyOf(i));
        if (attributed.size !== SHARED_TAGS || !sharedTags.every((tag) => attributed.has(tag))) {
          throw new Error(`key ${keyOf(i)} does not hold exactly the shared tags`);
        }
      }
      expect(map.get(keyOf(0))).toEqual(['live']);
      expect(map.get(keyOf(KEYS - 1))).toEqual(['live']);

      // Asserted last, so a run that is too slow still proves the result above.
      expect(elapsedMs).toBeLessThanOrEqual(SHARED_TAGS_BOUND_MS);
    },
    JEST_TIMEOUT_MS,
  );
});
