/**
 * LWWMap Micro-Benchmarks
 *
 * Measures performance of Last-Write-Wins Map operations:
 * - set(): Insert/update key-value pairs
 * - get(): Retrieve values by key
 * - merge(): Apply another replica's records, one per key
 */

import { bench, describe } from 'vitest';
import { LWWMap, HLC } from '../index';

const N = 10_000;

// Pre-populate map outside of benchmark
const hlc = new HLC('bench-node');
const map = new LWWMap<string, { value: number }>(hlc);
for (let i = 0; i < N; i++) {
  map.set(`key-${i}`, { value: i });
}

type BenchMap = LWWMap<string, { value: number }>;

// `LWWMap.merge` takes one remote record at a time, so applying another
// replica's state means merging each of its records under its key.
function mergeAll(target: BenchMap, delta: BenchMap): void {
  for (const key of delta.allKeys()) {
    const record = delta.getRecord(key);
    if (record) target.merge(key, record);
  }
}

describe('LWWMap', () => {
  bench('set() - existing key', () => {
    const key = `key-${Math.floor(Math.random() * N)}`;
    map.set(key, { value: Date.now() });
  });

  bench('set() - new key', () => {
    const key = `new-key-${Date.now()}-${Math.random()}`;
    map.set(key, { value: 42 });
  });

  bench('get() - existing key', () => {
    const key = `key-${Math.floor(Math.random() * N)}`;
    map.get(key);
  });

  bench('get() - missing key', () => {
    map.get('nonexistent-key');
  });

  bench('merge() - 100 updates', () => {
    const deltaHlc = new HLC('delta-node');
    const delta = new LWWMap<string, { value: number }>(deltaHlc);
    for (let i = 0; i < 100; i++) {
      delta.set(`key-${i}`, { value: i + 1000 });
    }
    mergeAll(map, delta);
  });

  bench('merge() - 10 updates', () => {
    const deltaHlc = new HLC('delta-node-small');
    const delta = new LWWMap<string, { value: number }>(deltaHlc);
    for (let i = 0; i < 10; i++) {
      delta.set(`key-${i}`, { value: i + 2000 });
    }
    mergeAll(map, delta);
  });
});
