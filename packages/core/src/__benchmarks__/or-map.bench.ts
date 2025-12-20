/**
 * ORMap Micro-Benchmarks
 *
 * Measures performance of Observed-Remove Map operations:
 * - add(): Add element to set
 * - remove(): Remove element from set
 * - get(): Retrieve all values for a key
 * - merge(): Combine two maps
 */

import { bench, describe } from 'vitest';
import { ORMap, HLC } from '../index';

const N = 5_000;

// Pre-populate map outside of benchmark
const hlc = new HLC('bench-ormap');
const map = new ORMap<string, { name: string }>(hlc);
for (let i = 0; i < N; i++) {
  map.add(`item-${i}`, { name: `Item ${i}` });
}

describe('ORMap', () => {
  bench('add() - new element', () => {
    const key = `new-item-${Date.now()}-${Math.random()}`;
    map.add(key, { name: 'New Item' });
  });

  bench('add() - existing key', () => {
    const key = `item-${Math.floor(Math.random() * N)}`;
    map.add(key, { name: 'Additional Value' });
  });

  bench('get() - existing key', () => {
    const key = `item-${Math.floor(Math.random() * N)}`;
    map.get(key);
  });

  bench('get() - missing key', () => {
    map.get('nonexistent-key');
  });

  bench('allKeys()', () => {
    map.allKeys();
  });

  bench('merge() - 50 updates', () => {
    const deltaHlc = new HLC('delta-ormap');
    const delta = new ORMap<string, { name: string }>(deltaHlc);
    for (let i = 0; i < 50; i++) {
      delta.add(`merge-${i}`, { name: `Merged ${i}` });
    }
    map.merge(delta);
  });

  bench('merge() - 10 updates', () => {
    const deltaHlc = new HLC('delta-ormap-small');
    const delta = new ORMap<string, { name: string }>(deltaHlc);
    for (let i = 0; i < 10; i++) {
      delta.add(`small-merge-${i}`, { name: `Small Merged ${i}` });
    }
    map.merge(delta);
  });
});
