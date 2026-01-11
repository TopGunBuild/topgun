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

const isQuickMode = process.env.BENCH_QUICK === 'true';

// Dataset sizes
const SMALL = 1_000;
const MEDIUM = 10_000;
const LARGE = isQuickMode ? 10_000 : 100_000;

// Pre-populate maps outside of benchmark
const smallMap = new SortedMap<number, string>(numericComparator);
const mediumMap = new SortedMap<number, string>(numericComparator);
const largeMap = isQuickMode ? mediumMap : new SortedMap<number, string>(numericComparator);

for (let i = 0; i < SMALL; i++) {
  smallMap.set(i, `value-${i}`);
}
for (let i = 0; i < MEDIUM; i++) {
  mediumMap.set(i, `value-${i}`);
}
if (!isQuickMode) {
  for (let i = 0; i < LARGE; i++) {
    largeMap.set(i, `value-${i}`);
  }
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

  if (!isQuickMode) {
    bench(`get() - 100K entries`, () => {
      const key = Math.floor(Math.random() * LARGE);
      largeMap.get(key);
    });
  }
});

describe('SortedMap - set() O(log N)', () => {
  bench(`set() - update existing key (${isQuickMode ? '10K' : '100K'})`, () => {
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
  const largeLabel = isQuickMode ? '10K' : '100K';

  bench(`range() - 10 results from ${largeLabel}`, () => {
    const start = Math.floor(Math.random() * (LARGE - 10));
    const results = [...largeMap.range(start, start + 10)];
    void results.length;
  });

  bench(`range() - 100 results from ${largeLabel}`, () => {
    const start = Math.floor(Math.random() * (LARGE - 100));
    const results = [...largeMap.range(start, start + 100)];
    void results.length;
  });

  bench(`range() - 1000 results from ${largeLabel}`, () => {
    const start = Math.floor(Math.random() * (LARGE - 1000));
    const results = [...largeMap.range(start, start + 1000)];
    void results.length;
  });

  if (!isQuickMode) {
    bench('range() - 10000 results from 100K', () => {
      const start = Math.floor(Math.random() * (LARGE - 10000));
      const results = [...largeMap.range(start, start + 10000)];
      void results.length;
    });
  }
});

describe('SortedMap - greaterThan/lessThan', () => {
  const midpoint = isQuickMode ? 5000 : 50000;
  const largeLabel = isQuickMode ? '10K' : '100K';

  bench(`greaterThan() - first 100 from ${largeLabel}`, () => {
    let count = 0;
    for (const entry of largeMap.greaterThan(midpoint)) {
      count++;
      if (count >= 100) break;
    }
  });

  bench(`lessThan() - first 100 from ${largeLabel}`, () => {
    let count = 0;
    for (const entry of largeMap.lessThan(midpoint)) {
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
  const largeLabel = isQuickMode ? '10K' : '100K';

  bench(`minKey() - ${largeLabel} entries`, () => {
    largeMap.minKey();
  });

  bench(`maxKey() - ${largeLabel} entries`, () => {
    largeMap.maxKey();
  });

  bench(`has() - existing key (${largeLabel})`, () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.has(key);
  });

  bench(`has() - missing key (${largeLabel})`, () => {
    largeMap.has(LARGE + 1000);
  });

  bench(`floorKey() - ${largeLabel} entries`, () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.floorKey(key);
  });

  bench(`ceilingKey() - ${largeLabel} entries`, () => {
    const key = Math.floor(Math.random() * LARGE);
    largeMap.ceilingKey(key);
  });
});

describe('SortedMap vs Map - comparison', () => {
  const largeLabel = isQuickMode ? '10K' : '100K';

  // Create equivalent native Map for comparison
  const nativeMap = new Map<number, string>();
  for (let i = 0; i < LARGE; i++) {
    nativeMap.set(i, `value-${i}`);
  }

  bench(`Native Map.get() - ${largeLabel} entries`, () => {
    const key = Math.floor(Math.random() * LARGE);
    nativeMap.get(key);
  });

  bench(`SortedMap.get() - ${largeLabel} entries`, () => {
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
