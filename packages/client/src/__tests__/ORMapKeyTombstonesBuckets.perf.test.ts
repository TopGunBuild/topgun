import { HLC, ORMap } from '@topgunbuild/core';
import {
  orMapKeyTombstonesBucketOf,
  serializeOrMapKeyTombstonesBucket,
} from '../utils/orMapKeyTombstones';
import type { PersistedKeyTombstones } from '../utils/orMapKeyTombstones';

/**
 * Cost bounds for writing one bucket of a persisted per-key tombstone
 * attribution.
 *
 * A local remove, and every sync response that changes a key's attribution,
 * rewrites the storage entry that key's attribution lives in. The shape that
 * makes this expensive is a map whose keys all share one large tombstone set
 * (left behind by a client that used to push its map-wide set with every key):
 * 1 000 keys x 1 000 tags is a million tags. Stored as a single entry that is
 * 38.9 MB of JSON, and an IndexedDB write has to structured-clone all of it
 * (about 60 ms) on every remove. Stored in buckets, one write carries only the
 * keys that share a bucket.
 *
 * These are regression guards, not benchmarks. Each time bound is more than
 * ten times the time measured for it (a 2021 laptop, under this test runner),
 * so a slower CI machine does not trip it, and the size bound is a ratio, so
 * it does not depend on the machine at all.
 */

const KEYS = 1_000;
const SHARED_TAGS = 1_000;

/**
 * The largest bucket against the map's whole attribution, in JSON bytes.
 * Measured: 389 038 bytes (10 keys) against 38 903 891, 1.0 %. The mean bucket
 * holds 1 000 / 256 keys, so 5 % leaves room for a different key set without
 * letting a return to one whole-map entry pass.
 */
const LARGEST_BUCKET_MAX_SHARE = 0.05;

/**
 * Structured clone of the largest bucket value, which is what an IndexedDB
 * write of it costs on the calling thread.
 * Measured: 0.28 ms (the whole attribution in one entry: 58 to 62 ms).
 * Bound: 20 ms.
 */
const LARGEST_BUCKET_CLONE_BOUND_MS = 20;

/**
 * Building the largest bucket's value from the map, as a local remove does
 * before it commits: one pass over every attributed key to find the bucket's
 * keys, then a copy of their tags.
 * Measured: 0.36 ms. Bound: 20 ms.
 */
const BUCKET_SERIALIZE_BOUND_MS = 20;

const keyOf = (i: number): string => `key-${i}`;

/** The fastest of several runs: the cost of the work, not of a GC pause next to it. */
function fastestOf(runs: number, work: () => void): number {
  let fastest = Number.POSITIVE_INFINITY;
  for (let i = 0; i < runs; i++) {
    const start = performance.now();
    work();
    fastest = Math.min(fastest, performance.now() - start);
  }
  return fastest;
}

describe('per-key tombstone attribution bucket cost', () => {
  test('one bucket of 1 000 keys sharing 1 000 tombstones is a small fraction of the whole and cheap to write', () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    // Tags as a client creates them (`millis:counter:nodeId`), with a node id
    // of the usual length: 36 characters each, 39 MB of JSON for the map.
    const sharedTags = Array.from(
      { length: SHARED_TAGS },
      (_, j) => `1700000000000:${j}:client-0123456789a`,
    );
    const keys = Array.from({ length: KEYS }, (_, i) => keyOf(i));
    for (const key of keys) map.add(key, 'live');
    map.addKeyTombstones(keys.map((key) => [key, sharedTags] as const));

    const buckets = Array.from(new Set(keys.map(orMapKeyTombstonesBucketOf)));
    const values = new Map<string, PersistedKeyTombstones>(
      buckets.map((bucket) => [bucket, serializeOrMapKeyTombstonesBucket(map, bucket)]),
    );

    // The buckets partition the attribution: every key in exactly one of them,
    // in the bucket its own id names, with all of its tags.
    const whole: PersistedKeyTombstones = [];
    for (const [bucket, pairs] of values) {
      for (const [key, tags] of pairs) {
        expect(orMapKeyTombstonesBucketOf(key)).toBe(bucket);
        expect(tags).toHaveLength(SHARED_TAGS);
      }
      whole.push(...pairs);
    }
    expect(whole.map(([key]) => key).sort()).toEqual([...keys].sort());
    expect(buckets.length).toBeLessThanOrEqual(256);

    let largestBucket = buckets[0];
    for (const bucket of buckets) {
      if ((values.get(bucket)?.length ?? 0) > (values.get(largestBucket)?.length ?? 0)) {
        largestBucket = bucket;
      }
    }
    const largest = values.get(largestBucket) ?? [];
    const wholeBytes = JSON.stringify(whole).length;
    const largestBytes = JSON.stringify(largest).length;

    const cloneMs = fastestOf(5, () => {
      structuredClone(largest);
    });
    const serializeMs = fastestOf(5, () => {
      serializeOrMapKeyTombstonesBucket(map, largestBucket);
    });

    expect(wholeBytes).toBeGreaterThan(30_000_000);
    expect(largestBytes / wholeBytes).toBeLessThanOrEqual(LARGEST_BUCKET_MAX_SHARE);
    expect(cloneMs).toBeLessThanOrEqual(LARGEST_BUCKET_CLONE_BOUND_MS);
    expect(serializeMs).toBeLessThanOrEqual(BUCKET_SERIALIZE_BOUND_MS);
  }, 120_000);
});
