/**
 * SortedMap Micro-Benchmarks
 *
 * Measures performance of SortedMap (B+Tree) operations:
 * - set(): Insert/update key-value pairs - O(log N)
 * - get(): Retrieve values by key - O(log N)
 * - delete(): Remove key-value pairs - O(log N)
 * - range(): Range queries - O(log N + K)
 * - greaterThan()/lessThan(): Bound queries
 */

import { bench, describe } from 'vitest';
import { SortedMap, numericComparator } from '../query/ds';

// Dataset sizes
const SMALL = 1_000;
const MEDIUM = 10_000;
const LARGE = 100_000;

// Pre-populate maps outside of benchmark
const smallMap = new SortedMap<number, string>(numericComparator);
const mediumMap = new SortedMap<number, string>(numericComparator);
const largeMap = new SortedMap<number, string>(numericComparator);

for (let i = 0; i < SMALL; i++) {
  smallMap.set(i, `value-${i}`);
}
for (let i = 0; i < MEDIUM; i++) {
  mediumMap.set(i, `value-${i}`);
}
for (let i = 0; i < LARGE; i++) {
  largeMap.set(i, `value-${i}`);
}

describe('SortedMap - get() O(log N)', () => {
  bench(`get() - 1K entries`, () => {
    const key = Math.floor(Math.random() * SMALL);
    smallMap.get(key);
  });

  bench(`get() - 10K entries`, () => {
    const key = Math.floor(Math.random() * MEDIUM);
    mediumMap.get(key);
  });

  bench(`get() - 100K entries`, () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.get(key);
  });
});

describe('SortedMap - set() O(log N)', () => {
  bench('set() - update existing key (100K)', () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.set(key, `updated-${Date.now()}`);
  });

  bench('set() - new key (temporary map)', () => {
    const tempMap = new SortedMap<number, string>(numericComparator);
    for (let i = 0; i < 100; i++) {
      tempMap.set(i, `value-${i}`);
    }
  });
});

describe('SortedMap - range() O(log N + K)', () => {
  bench('range() - 10 results from 100K', () => {
    const start = Math.floor(Math.random() * (LARGE - 10));
    const results = [...largeMap.range(start, start + 10)];
    void results.length;
  });

  bench('range() - 100 results from 100K', () => {
    const start = Math.floor(Math.random() * (LARGE - 100));
    const results = [...largeMap.range(start, start + 100)];
    void results.length;
  });

  bench('range() - 1000 results from 100K', () => {
    const start = Math.floor(Math.random() * (LARGE - 1000));
    const results = [...largeMap.range(start, start + 1000)];
    void results.length;
  });

  bench('range() - 10000 results from 100K', () => {
    const start = Math.floor(Math.random() * (LARGE - 10000));
    const results = [...largeMap.range(start, start + 10000)];
    void results.length;
  });
});

describe('SortedMap - greaterThan/lessThan', () => {
  bench('greaterThan() - first 100 from 100K', () => {
    let count = 0;
    for (const entry of largeMap.greaterThan(50000)) {
      count++;
      if (count >= 100) break;
    }
  });

  bench('lessThan() - first 100 from 100K', () => {
    let count = 0;
    for (const entry of largeMap.lessThan(50000)) {
      count++;
      if (count >= 100) break;
    }
  });
});

describe('SortedMap - iteration', () => {
  bench('entries() - iterate 1K entries', () => {
    let count = 0;
    for (const entry of smallMap.entries()) {
      count++;
    }
  });

  bench('keys() - iterate 1K keys', () => {
    let count = 0;
    for (const key of smallMap.keys()) {
      count++;
    }
  });

  bench('entriesReversed() - iterate 1K entries', () => {
    let count = 0;
    for (const entry of smallMap.entriesReversed()) {
      count++;
    }
  });
});

describe('SortedMap - utility operations', () => {
  bench('minKey() - 100K entries', () => {
    largeMap.minKey();
  });

  bench('maxKey() - 100K entries', () => {
    largeMap.maxKey();
  });

  bench('has() - existing key (100K)', () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.has(key);
  });

  bench('has() - missing key (100K)', () => {
    largeMap.has(LARGE + 1000);
  });

  bench('floorKey() - 100K entries', () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.floorKey(key);
  });

  bench('ceilingKey() - 100K entries', () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.ceilingKey(key);
  });
});

describe('SortedMap vs Map - comparison', () => {
  // Create equivalent native Map for comparison
  const nativeMap = new Map<number, string>();
  for (let i = 0; i < LARGE; i++) {
    nativeMap.set(i, `value-${i}`);
  }

  bench('Native Map.get() - 100K entries', () => {
    const key = Math.floor(Math.random() * LARGE);
    nativeMap.get(key);
  });

  bench('SortedMap.get() - 100K entries', () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.get(key);
  });

  bench('Native Map.set() - existing key', () => {
    const key = Math.floor(Math.random() * LARGE);
    nativeMap.set(key, `updated-${Date.now()}`);
  });

  bench('SortedMap.set() - existing key', () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.set(key, `updated-${Date.now()}`);
  });
});
