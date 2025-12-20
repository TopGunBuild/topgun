/**
 * LWWMap Micro-Benchmarks
 *
 * Measures performance of Last-Write-Wins Map operations:
 * - set(): Insert/update key-value pairs
 * - get(): Retrieve values by key
 * - merge(): Combine two maps
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
    map.merge(delta);
  });

  bench('merge() - 10 updates', () => {
    const deltaHlc = new HLC('delta-node-small');
    const delta = new LWWMap<string, { value: number }>(deltaHlc);
    for (let i = 0; i < 10; i++) {
      delta.set(`key-${i}`, { value: i + 2000 });
    }
    map.merge(delta);
  });
});
