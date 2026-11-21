/**
 * QueryHandle under a burst of live updates: the event-loop block of a burst
 * and its flush against a fixed budget, and hardware-independent proof that one
 * update costs O(log N) — no untouched row is read, the tree does O(1)
 * operations per update and no iteration, and the comparator is called
 * O(log N) times.
 *
 * Real timers throughout: the budget is the event loop's own measurement.
 */

import { monitorEventLoopDelay } from 'perf_hooks';
import { SortedMap } from '@topgunbuild/core';
import { QueryHandle, QueryFilter, QueryResultItem } from '../QueryHandle';
import type { SyncEngine } from '../SyncEngine';

// Counts comparator calls without a hook in production code: every SortedMap
// the handle builds wraps its comparator in a counter.
jest.mock('@topgunbuild/core', () => {
  const actual = jest.requireActual('@topgunbuild/core');
  class CountingSortedMap<K, V> extends actual.SortedMap<K, V> {
    static comparatorCalls = 0;
    constructor(comparator?: (a: K, b: K) => number) {
      super(
        comparator &&
          ((a: K, b: K) => {
            CountingSortedMap.comparatorCalls++;
            return comparator(a, b);
          }),
      );
    }
  }
  return { ...actual, SortedMap: CountingSortedMap };
});

/** Event-loop block budget, fixed before measurement. */
const BUDGET_MS = 50;

type Row = { score: number; tag?: string };

const makeEngine = () =>
  ({
    subscribeToQuery: () => {},
    unsubscribeFromQuery: () => {},
    runLocalQuery: async () => [],
    getRecordSyncStateTracker: () => ({ onChange: () => () => {}, get: () => 'synced' }),
  }) as unknown as SyncEngine;

const nextTurn = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const nextImmediate = () => new Promise<void>((resolve) => setImmediate(resolve));

/** Deterministic PRNG, so every run applies the same mix. */
function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Op = { key: string; value: Row | null };

/**
 * N seed rows and M updates: 70 % updates that move an existing row, 15 %
 * ENTERs of new keys, 15 % LEAVEs of existing keys. Also returns the rows the
 * updates leave, computed without the handle.
 */
function workload(n: number, m: number, seed: number) {
  const rand = mulberry32(seed);
  const score = () => Math.floor(rand() * 1_000_000);
  const present: string[] = [];
  const state = new Map<string, Row>();
  const seedRows: { key: string; value: Row }[] = [];
  for (let i = 0; i < n; i++) {
    const key = `k${i}`;
    const value = { score: score() };
    present.push(key);
    state.set(key, value);
    seedRows.push({ key, value });
  }
  const ops: Op[] = [];
  let next = n;
  for (let i = 0; i < m; i++) {
    const r = rand();
    if (r < 0.15) {
      const key = `k${next++}`;
      const value = { score: score() };
      present.push(key);
      state.set(key, value);
      ops.push({ key, value });
    } else if (r < 0.3) {
      const at = Math.floor(rand() * present.length);
      const key = present[at];
      present[at] = present[present.length - 1];
      present.pop();
      state.delete(key);
      ops.push({ key, value: null });
    } else {
      const key = present[Math.floor(rand() * present.length)];
      const value = { score: score() };
      state.set(key, value);
      ops.push({ key, value });
    }
  }
  return { seedRows, ops, state };
}

/** The expected order, built without the handle: score ascending, then the key's UTF-8 bytes. */
function referenceOrder(state: Map<string, Row>, sorted: boolean): string[] {
  const keys = [...state.keys()];
  const byKey = (a: string, b: string) => Buffer.compare(Buffer.from(a), Buffer.from(b));
  return keys.sort((a, b) =>
    sorted ? state.get(a)!.score - state.get(b)!.score || byKey(a, b) : byKey(a, b),
  );
}

type BurstResult = {
  burstMs: number;
  flushMs: number;
  maxDelayMs: number;
  emissions: number;
  deltaCalls: number;
};

/**
 * Seeds the handle, waits an idle turn, then applies every op inside one
 * setImmediate callback and lets the flush and one more turn pass, measuring
 * the largest event-loop delay over that window.
 */
async function runBurst(
  handle: QueryHandle<Row>,
  seedRows: { key: string; value: Row }[],
  ops: Op[],
): Promise<BurstResult> {
  handle.onResult(seedRows, 'server');
  const gc = (global as { gc?: () => void }).gc;
  gc?.();
  await nextTurn();
  const histogram = monitorEventLoopDelay({ resolution: 1 });
  histogram.enable();
  let burstMs = 0;
  let flushStart = 0;
  let flushEnd = 0;
  let emissions = 0;
  let deltaCalls = 0;
  handle.onDelta(() => {
    flushStart = performance.now();
    deltaCalls++;
  });
  handle.subscribe(() => {
    flushEnd = performance.now();
    emissions++;
  });
  await new Promise<void>((resolve) =>
    setImmediate(() => {
      const t0 = performance.now();
      for (const op of ops) handle.onUpdate(op.key, op.value);
      burstMs = performance.now() - t0;
      resolve();
    }),
  );
  await nextTurn();
  await nextTurn();
  await nextImmediate();
  histogram.disable();
  return {
    burstMs,
    flushMs: flushEnd - flushStart,
    maxDelayMs: histogram.max / 1e6,
    emissions,
    deltaCalls,
  };
}

describe('QueryHandle burst', () => {
  const gcAvailable = typeof (global as { gc?: () => void }).gc === 'function';

  describe('1-2. event-loop block of a burst and its flush', () => {
    const cells: { n: number; m: number; filter: QueryFilter; label: string }[] = [];
    for (const n of [1_000, 5_000, 20_000]) {
      for (const m of [100, 1_000]) {
        cells.push({ n, m, filter: { sort: { score: 'asc' } }, label: 'sorted' });
      }
    }
    cells.push({
      n: 5_000,
      m: 1_000,
      filter: { sort: { score: 'asc' }, limit: 100 },
      label: 'limit 100',
    });
    cells.push({ n: 5_000, m: 1_000, filter: {}, label: 'unsorted' });

    test.each(cells)('N=$n M=$m $label', async ({ n, m, filter, label }) => {
      const { seedRows, ops, state } = workload(n, m, n + m);
      const handle = new QueryHandle<Row>(makeEngine(), 'm', filter);
      let received: QueryResultItem<Row>[] = [];
      handle.subscribe((rows) => (received = rows));
      const result = await runBurst(handle, seedRows, ops);
      // One line per cell for the execution record, printed before any assertion.
      process.stdout.write(
        `BURST N=${n} M=${m} ${label} burst_ms=${result.burstMs.toFixed(2)} flush_ms=${result.flushMs.toFixed(2)} max_loop_delay_ms=${result.maxDelayMs.toFixed(2)} gc=${gcAvailable}\n`,
      );
      expect(result.maxDelayMs).toBeLessThanOrEqual(BUDGET_MS);
      expect(result.emissions).toBe(1);
      expect(result.deltaCalls).toBe(1);
      const expected = referenceOrder(state, filter.sort !== undefined);
      const shown = filter.limit ? expected.slice(0, filter.limit) : expected;
      expect(received.map((r) => r._key)).toEqual(shown);
      expect(received.map((r) => r.score)).toEqual(shown.map((k) => state.get(k)!.score));
    });
  });

  test('3. an update reads no other row', async () => {
    const n = 5_000;
    const reads = new Map<string, number>();
    const counted = (key: string, value: Row): Row => {
      reads.set(key, 0);
      const bump = () => reads.set(key, reads.get(key)! + 1);
      return new Proxy(value, {
        get(target, prop, receiver) {
          bump();
          return Reflect.get(target, prop, receiver);
        },
        has(target, prop) {
          bump();
          return Reflect.has(target, prop);
        },
        ownKeys(target) {
          bump();
          return Reflect.ownKeys(target);
        },
        getOwnPropertyDescriptor(target, prop) {
          bump();
          return Reflect.getOwnPropertyDescriptor(target, prop);
        },
      });
    };
    const handle = new QueryHandle<Row>(makeEngine(), 'm', { sort: { score: 'asc' } });
    handle.subscribe(() => {});
    handle.onDelta(() => {});
    handle.onResult(
      Array.from({ length: n }, (_, i) => ({
        key: `k${i}`,
        value: counted(`k${i}`, { score: i }),
      })),
      'server',
    );
    for (const key of reads.keys()) reads.set(key, 0);
    const touched = new Set<string>();
    for (let i = 0; i < 1_000; i++) {
      const key = `k${(i * 7) % n}`;
      touched.add(key);
      handle.onUpdate(key, { score: n - i });
    }
    expect(touched.size).toBe(1_000);
    await nextTurn();
    let untouchedReads = 0;
    for (const [key, count] of reads) if (!touched.has(key)) untouchedReads += count;
    expect(untouchedReads).toBe(0);
  });

  test('4. an update does O(1) tree operations and no iteration', async () => {
    const proto = SortedMap.prototype as unknown as Record<string | symbol, unknown>;
    const iterationNames: (string | symbol)[] = [
      'entries',
      'entriesReversed',
      'values',
      'keys',
      'forEach',
      Symbol.iterator,
      'range',
      'greaterThan',
      'lessThan',
      'at',
      'higherKey',
      'ceilingKey',
      'lowerKey',
      'floorKey',
      'minKey',
      'maxKey',
      'getOrSet',
      'update',
      'clear',
    ];
    for (const name of iterationNames) expect(typeof proto[name]).toBe('function');
    const spy = (name: string | symbol) =>
      jest.spyOn(proto as Record<string | symbol, () => unknown>, name as string);
    const set = spy('set');
    const del = spy('delete');
    const iteration = iterationNames.map(spy);
    const iterationCalls = () => iteration.reduce((sum, s) => sum + s.mock.calls.length, 0);

    const handle = new QueryHandle<Row>(makeEngine(), 'm', { sort: { score: 'asc' } });
    handle.subscribe(() => {});
    handle.onResult(
      Array.from({ length: 1_000 }, (_, i) => ({ key: `k${i}`, value: { score: i } })),
      'server',
    );
    const rows = (handle as unknown as { rows: Map<string, unknown> }).rows;
    const rowIteration = (['keys', 'values', 'entries', 'forEach', Symbol.iterator] as const).map(
      (name) =>
        jest.spyOn(rows as unknown as Record<string | symbol, () => unknown>, name as string),
    );
    const rowIterationCalls = () => rowIteration.reduce((sum, s) => sum + s.mock.calls.length, 0);

    const counts = (apply: () => void) => {
      for (const s of [set, del, ...iteration, ...rowIteration]) s.mockClear();
      apply();
      return {
        set: set.mock.calls.length,
        delete: del.mock.calls.length,
        iteration: iterationCalls(),
        rowIteration: rowIterationCalls(),
      };
    };
    const none = { iteration: 0, rowIteration: 0 };
    expect(counts(() => handle.onUpdate('new', { score: 5.5 }))).toEqual({
      set: 1,
      delete: 0,
      ...none,
    });
    expect(counts(() => handle.onUpdate('k3', null))).toEqual({ set: 0, delete: 1, ...none });
    expect(counts(() => handle.onUpdate('k10', { score: 999.5 }))).toEqual({
      set: 1,
      delete: 1,
      ...none,
    });
    const still = counts(() => handle.onUpdate('k20', { score: 20, tag: 'x' }));
    expect(still.set).toBeLessThanOrEqual(1);
    expect(still).toMatchObject({ delete: 0, ...none });

    for (const s of iteration) s.mockClear();
    await nextTurn();
    expect(iterationCalls()).toBe(1);
    for (const s of [set, del, ...iteration, ...rowIteration]) s.mockRestore();
  });

  test('5. comparator calls per update are O(log N), with sync state observed', async () => {
    const Counting = SortedMap as unknown as { comparatorCalls: number };
    const measure = async (n: number) => {
      const { seedRows, ops } = workload(n, 1_000, 7);
      const handle = new QueryHandle<Row>(makeEngine(), 'm', { sort: { score: 'asc' } });
      handle.subscribe(() => {});
      handle.onSyncStateChange(() => {});
      handle.onResult(seedRows, 'server');
      expect(handle.getSnapshotMeta().hasEmitted).toBe(true);
      const internals = handle as unknown as Record<string, () => unknown>;
      const syncState = jest.spyOn(internals, 'computeFilteredSyncState');
      const snapshot = jest.spyOn(internals, 'getSnapshot');
      const perUpdate: number[] = [];
      for (const op of ops) {
        const before = Counting.comparatorCalls;
        handle.onUpdate(op.key, op.value);
        perUpdate.push(Counting.comparatorCalls - before);
      }
      expect(syncState).not.toHaveBeenCalled();
      expect(snapshot).not.toHaveBeenCalled();
      await nextTurn();
      expect(syncState.mock.calls.length).toBeLessThanOrEqual(1);
      expect(snapshot.mock.calls.length).toBeLessThanOrEqual(1);
      const max = Math.max(...perUpdate);
      const mean = perUpdate.reduce((a, b) => a + b, 0) / perUpdate.length;
      return { max, mean, bound: 4 * Math.ceil(Math.log2(n)) };
    };
    const small = await measure(5_000);
    const large = await measure(20_000);
    process.stdout.write(
      `COMPARATOR N=5000 max=${small.max} mean=${small.mean.toFixed(2)} bound=${small.bound}; N=20000 max=${large.max} mean=${large.mean.toFixed(2)} bound=${large.bound}; mean_ratio=${(large.mean / small.mean).toFixed(3)}\n`,
    );
    expect(small.max).toBeLessThanOrEqual(small.bound);
    expect(large.max).toBeLessThanOrEqual(large.bound);
    expect(large.mean / small.mean).toBeLessThanOrEqual(1.5);
  });

  test('6. a listener that mutates its array changes neither getSnapshot() nor later emissions', async () => {
    const handle = new QueryHandle<Row>(makeEngine(), 'm', { sort: { score: 'asc' } });
    const other: string[][] = [];
    handle.subscribe((rows) => {
      rows.sort((a, b) => b.score - a.score);
      rows.push({ score: -1, _key: 'intruder' });
    });
    handle.subscribe((rows) => other.push(rows.map((r) => r._key)));
    handle.onResult(
      [
        { key: 'a', value: { score: 1 } },
        { key: 'b', value: { score: 2 } },
      ],
      'server',
    );
    expect(handle.getSnapshot().map((r) => r._key)).toEqual(['a', 'b']);
    handle.onUpdate('c', { score: 3 });
    await nextTurn();
    expect(handle.getSnapshot().map((r) => r._key)).toEqual(['a', 'b', 'c']);
    // The first entry is the immediate call a non-first listener gets on subscribe.
    expect(other).toEqual([[], ['a', 'b'], ['a', 'b', 'c']]);
  });
});
