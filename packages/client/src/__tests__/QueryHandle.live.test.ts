/**
 * QueryHandle live delivery: per-key change computation, one emission per timer
 * turn for live updates, synchronous delivery of snapshots, and the server's
 * sort-key order. One test per edge case.
 */

import { QueryHandle, QueryFilter, QueryResultItem } from '../QueryHandle';
import type { ChangeEvent } from '../ChangeTracker';
import { SyncEngine } from '../SyncEngine';

type Row = Record<string, unknown>;

const makeEngine = () =>
  ({
    subscribeToQuery: jest.fn(),
    unsubscribeFromQuery: jest.fn(),
    runLocalQuery: jest.fn().mockResolvedValue([]),
    getRecordSyncStateTracker: () => ({ onChange: () => () => {}, get: () => 'synced' }),
  }) as unknown as SyncEngine;

const nextTurn = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/** A handle with one subscribe listener and one onDelta listener that record every call. */
function setup(filter: QueryFilter = {}) {
  const handle = new QueryHandle<Row>(makeEngine(), 'm', filter);
  const emissions: { rows: QueryResultItem<Row>[]; settled?: boolean }[] = [];
  const deltas: ChangeEvent<Row>[][] = [];
  handle.subscribe((rows, meta) => emissions.push({ rows, settled: meta?.settled }));
  handle.onDelta((changes) => deltas.push(changes));
  const keys = () => handle.getSnapshot().map((r) => r._key);
  const lastKeys = () => emissions[emissions.length - 1].rows.map((r) => r._key);
  return { handle, emissions, deltas, keys, lastKeys };
}

const seed = (handle: QueryHandle<Row>, rows: [string, Row][]) =>
  handle.onResult(
    rows.map(([key, value]) => ({ key, value })),
    'server',
  );

const brief = (changes: ChangeEvent<Row>[]) =>
  changes.map((c) => [c.type, c.key, c.value ?? null, c.previousValue ?? null]);

describe('QueryHandle live delivery', () => {
  describe('order', () => {
    test('an unsorted query is ordered by key', async () => {
      const { handle, keys, lastKeys } = setup();
      seed(handle, [
        ['b', { v: 1 }],
        ['c', { v: 2 }],
        ['a', { v: 3 }],
      ]);
      expect(keys()).toEqual(['a', 'b', 'c']);
      handle.onUpdate('ab', { v: 0 });
      await nextTurn();
      expect(lastKeys()).toEqual(['a', 'ab', 'b', 'c']);
    });

    test('a sorted query is ordered by the sort field, then by key', async () => {
      const { handle, keys, lastKeys } = setup({ sort: { n: 'asc' } });
      seed(handle, [
        ['a', { n: 3 }],
        ['b', { n: 1 }],
        ['c', { n: 2 }],
      ]);
      expect(keys()).toEqual(['b', 'c', 'a']);
      handle.onUpdate('a', { n: 0 });
      await nextTurn();
      expect(lastKeys()).toEqual(['a', 'b', 'c']);
    });

    test('ties are broken by key, and every tied row is kept', () => {
      const { handle, keys } = setup({ sort: { n: 'asc' } });
      seed(handle, [
        ['c', { n: 1 }],
        ['a', { n: 1 }],
        ['d', { n: 0 }],
        ['b', { n: 1 }],
      ]);
      expect(keys()).toEqual(['d', 'a', 'b', 'c']);
      // A desc field reverses the values, never the key tie-break.
      const desc = setup({ sort: { n: 'desc' } });
      seed(desc.handle, [
        ['c', { n: 1 }],
        ['a', { n: 1 }],
        ['d', { n: 0 }],
      ]);
      expect(desc.keys()).toEqual(['a', 'c', 'd']);
    });

    test('null and missing values come last under asc and first under desc', () => {
      const rows: [string, Row][] = [
        ['n', { n: null }],
        ['x', {}],
        ['one', { n: 1 }],
        ['two', { n: 2 }],
      ];
      const asc = setup({ sort: { n: 'asc' } });
      seed(asc.handle, rows);
      expect(asc.keys()).toEqual(['one', 'two', 'n', 'x']);
      const desc = setup({ sort: { n: 'desc' } });
      seed(desc.handle, rows);
      expect(desc.keys()).toEqual(['n', 'x', 'two', 'one']);
    });

    test('booleans, then numbers, then strings, then everything else', () => {
      const { handle, keys } = setup({ sort: { n: 'asc' } });
      seed(handle, [
        ['date', { n: new Date(0) }],
        ['str', { n: '1' }],
        ['obj', { n: { a: 1 } }],
        ['num', { n: 5 }],
        ['t', { n: true }],
        ['f', { n: false }],
        ['arr', { n: [1] }],
        ['neg', { n: -5 }],
      ]);
      // Date, object and array rank with null/missing, so among them the key decides.
      expect(keys()).toEqual(['f', 't', 'neg', 'num', 'str', 'arr', 'date', 'obj']);
    });

    test('NaN sorts after every other number and before strings; bigint compares with number', () => {
      const { handle, keys } = setup({ sort: { n: 'asc' } });
      seed(handle, [
        ['nan2', { n: NaN }],
        ['str', { n: 'a' }],
        ['big', { n: BigInt(Number.MAX_SAFE_INTEGER) * BigInt(4) }],
        ['nan1', { n: NaN }],
        ['three', { n: 3 }],
        ['bigSmall', { n: BigInt(2) }],
        ['inf', { n: Infinity }],
      ]);
      expect(keys()).toEqual(['bigSmall', 'three', 'big', 'inf', 'nan1', 'nan2', 'str']);
    });

    test('strings compare in code-point order, not UTF-16 code-unit order', () => {
      const { handle, keys } = setup({ sort: { s: 'asc' } });
      // U+1F600 is the surrogate pair D83D DE00; U+FFFD is one code unit FFFD.
      // Code units put the emoji first (D83D < FFFD); code points put it last.
      seed(handle, [
        ['emoji', { s: '\u{1F600}' }],
        ['replacement', { s: '�' }],
        ['plain', { s: 'z' }],
      ]);
      expect(keys()).toEqual(['plain', 'replacement', 'emoji']);
      // The key tie-break uses the same order.
      const unsorted = setup();
      seed(unsorted.handle, [
        ['\u{1F600}', {}],
        ['�', {}],
      ]);
      expect(unsorted.keys()).toEqual(['�', '\u{1F600}']);
    });

    test('a QUERY_UPDATE the client would rank elsewhere keeps the server row set', async () => {
      const { handle, emissions } = setup({ sort: { n: 'asc' } });
      seed(handle, [
        ['a', { n: 1 }],
        ['b', { n: 2 }],
      ]);
      // The server ranks a boolean differently; the client places the row by its
      // own order, and the set stays exactly the server's keys.
      handle.onUpdate('c', { n: true });
      handle.onUpdate('a', { n: 'x' });
      handle.onUpdate('a', { n: 0 });
      await nextTurn();
      const rows = emissions[emissions.length - 1].rows;
      expect(rows.map((r) => r._key)).toEqual(['c', 'a', 'b']);
      expect(new Set(rows.map((r) => r._key)).size).toBe(rows.length);
    });
  });

  describe('limit', () => {
    const seedWindow = () => {
      const t = setup({ sort: { n: 'asc' }, limit: 3 });
      seed(t.handle, [
        ['a', { n: 1 }],
        ['b', { n: 2 }],
        ['c', { n: null }],
      ]);
      return t;
    };

    test('a row entering above the window is held but not shown', async () => {
      const t = seedWindow();
      t.handle.onUpdate('d', { n: null });
      await nextTurn();
      // d ties with c on the null value; the key puts it after c, outside the window.
      expect(t.lastKeys()).toEqual(['a', 'b', 'c']);
    });

    test('a row entering inside the window displaces the last one', async () => {
      const t = seedWindow();
      t.handle.onUpdate('a2', { n: 1 });
      await nextTurn();
      expect(t.lastKeys()).toEqual(['a', 'a2', 'b']);
    });

    test('a row leaving from inside the window promotes the next', async () => {
      const t = seedWindow();
      t.handle.onUpdate('d', { n: null });
      t.handle.onUpdate('a', null);
      await nextTurn();
      expect(t.lastKeys()).toEqual(['b', 'c', 'd']);
    });
  });

  describe('coalescing and change events', () => {
    test('a key updated twice in one turn gives two update events, in order', async () => {
      const { handle, deltas, emissions } = setup();
      seed(handle, [['a', { v: 1 }]]);
      const before = emissions.length;
      handle.onUpdate('a', { v: 2 });
      handle.onUpdate('a', { v: 3 });
      expect(deltas).toHaveLength(1);
      await nextTurn();
      expect(deltas).toHaveLength(2);
      expect(brief(deltas[1])).toEqual([
        ['update', 'a', { v: 2 }, { v: 1 }],
        ['update', 'a', { v: 3 }, { v: 2 }],
      ]);
      expect(emissions).toHaveLength(before + 1);
      expect(emissions[emissions.length - 1].rows).toEqual([{ v: 3, _key: 'a' }]);
    });

    test('update then LEAVE in one turn gives update then remove', async () => {
      const { handle, deltas, emissions } = setup();
      seed(handle, [['a', { v: 1 }]]);
      handle.onUpdate('a', { v: 2 });
      handle.onUpdate('a', null);
      await nextTurn();
      expect(brief(deltas[1])).toEqual([
        ['update', 'a', { v: 2 }, { v: 1 }],
        ['remove', 'a', null, { v: 2 }],
      ]);
      expect(emissions[emissions.length - 1].rows).toEqual([]);
    });

    test('ENTER then LEAVE in one turn gives add then remove', async () => {
      const { handle, deltas, emissions } = setup();
      seed(handle, [['a', { v: 1 }]]);
      handle.onUpdate('b', { v: 2 });
      handle.onUpdate('b', null);
      await nextTurn();
      expect(brief(deltas[1])).toEqual([
        ['add', 'b', { v: 2 }, null],
        ['remove', 'b', null, { v: 2 }],
      ]);
      expect(emissions[emissions.length - 1].rows.map((r) => r._key)).toEqual(['a']);
    });

    test('LEAVE of an absent key records no event and emits nothing', async () => {
      const { handle, deltas, emissions } = setup();
      seed(handle, [['a', { v: 1 }]]);
      const pending = handle.getPendingChanges().length;
      const before = emissions.length;
      handle.onUpdate('zz', null);
      await nextTurn();
      expect(handle.getPendingChanges()).toHaveLength(pending);
      expect(deltas).toHaveLength(1);
      expect(emissions).toHaveLength(before);
    });

    test('a deep-equal update records no event, emits nothing and keeps the row object', async () => {
      const { handle, deltas, emissions } = setup();
      seed(handle, [['a', { v: 1, nested: { x: [1, 2] } }]]);
      const row = handle.getSnapshot()[0];
      const snapshot = handle.getSnapshot();
      const pending = handle.getPendingChanges().length;
      const before = emissions.length;
      handle.onUpdate('a', { v: 1, nested: { x: [1, 2] } });
      await nextTurn();
      expect(handle.getPendingChanges()).toHaveLength(pending);
      expect(deltas).toHaveLength(1);
      expect(emissions).toHaveLength(before);
      expect(handle.getSnapshot()).toBe(snapshot);
      expect(handle.getSnapshot()[0]).toBe(row);
    });

    test('a key twice in one items array gives add then update', () => {
      const { handle, deltas } = setup();
      handle.onResult(
        [
          { key: 'a', value: { v: 1 } },
          { key: 'a', value: { v: 2 } },
        ],
        'server',
      );
      expect(brief(deltas[0])).toEqual([
        ['add', 'a', { v: 1 }, null],
        ['update', 'a', { v: 2 }, { v: 1 }],
      ]);
      expect(handle.getSnapshot()).toEqual([{ v: 2, _key: 'a' }]);
    });

    test('resetChangeTracker then onUpdate reports every row as added', async () => {
      const { handle, deltas } = setup();
      seed(handle, [
        ['a', { v: 1 }],
        ['b', { v: 2 }],
      ]);
      handle.resetChangeTracker();
      handle.onUpdate('a', { v: 9 });
      expect(brief(handle.getPendingChanges())).toEqual([
        ['add', 'a', { v: 9 }, null],
        ['add', 'b', { v: 2 }, null],
      ]);
      await nextTurn();
      expect(brief(deltas[1])).toEqual(brief(handle.getPendingChanges()));
      // The flag applies once.
      handle.onUpdate('b', { v: 3 });
      expect(handle.getLastChange()).toMatchObject({ type: 'update', key: 'b' });
    });

    test('after resetChangeTracker, removing the last row still emits the empty rows', async () => {
      const { handle, emissions, lastKeys } = setup();
      seed(handle, [['a', { v: 1 }]]);
      await nextTurn();
      handle.resetChangeTracker();
      const before = emissions.length;
      handle.onUpdate('a', null);
      await nextTurn();
      expect(emissions.length).toBe(before + 1);
      expect(lastKeys()).toEqual([]);
    });

    test('resetChangeTracker delivers the undelivered changes to onDelta first', async () => {
      const { handle, deltas } = setup();
      seed(handle, [
        ['a', { v: 1 }],
        ['b', { v: 2 }],
      ]);
      handle.onUpdate('a', null);
      handle.resetChangeTracker();
      await nextTurn();
      expect(brief(deltas[deltas.length - 1])).toEqual([['remove', 'a', null, { v: 1 }]]);
      expect(handle.getPendingChanges()).toEqual([]);
    });
  });

  describe('snapshots', () => {
    test('a snapshot arriving with undelivered updates delivers them first, in one array', async () => {
      const { handle, deltas, emissions } = setup();
      seed(handle, [
        ['a', { v: 1 }],
        ['b', { v: 1 }],
      ]);
      handle.onUpdate('a', { v: 2 });
      handle.onUpdate('c', { v: 1 });
      const before = emissions.length;
      seed(handle, [
        ['a', { v: 3 }],
        ['d', { v: 1 }],
      ]);
      // Delivered synchronously, before onResult returned.
      expect(deltas).toHaveLength(2);
      expect(brief(deltas[1])).toEqual([
        ['update', 'a', { v: 2 }, { v: 1 }],
        ['add', 'c', { v: 1 }, null],
        ['update', 'a', { v: 3 }, { v: 2 }],
        ['add', 'd', { v: 1 }, null],
        ['remove', 'b', null, { v: 1 }],
        ['remove', 'c', null, { v: 1 }],
      ]);
      expect(emissions).toHaveLength(before + 1);
      expect(emissions[emissions.length - 1].rows.map((r) => r._key)).toEqual(['a', 'd']);
      await nextTurn();
      expect(deltas).toHaveLength(2);
      expect(emissions).toHaveLength(before + 1);
    });

    test('an overflow resubscribe replacing every row gives removes and adds in one emission', () => {
      const { handle, deltas, emissions } = setup();
      seed(handle, [
        ['a', { v: 1 }],
        ['b', { v: 1 }],
      ]);
      const before = emissions.length;
      seed(handle, [
        ['c', { v: 1 }],
        ['d', { v: 1 }],
      ]);
      expect(brief(deltas[1])).toEqual([
        ['add', 'c', { v: 1 }, null],
        ['add', 'd', { v: 1 }, null],
        ['remove', 'a', null, { v: 1 }],
        ['remove', 'b', null, { v: 1 }],
      ]);
      expect(emissions).toHaveLength(before + 1);
      expect(emissions[emissions.length - 1].rows.map((r) => r._key)).toEqual(['c', 'd']);
    });

    test('a whenSettled() awaiter sees the snapshot rows', async () => {
      // The loadMore path: rows are read from the subscribe callback once settled.
      const handle = new QueryHandle<Row>(makeEngine(), 'm', {});
      let seen: QueryResultItem<Row>[] = [];
      let deltaSeen: ChangeEvent<Row>[] = [];
      handle.subscribe((rows) => (seen = rows));
      handle.onDelta((changes) => (deltaSeen = changes));
      const observed = handle.whenSettled().then(() => ({
        rows: seen.map((r) => r._key),
        deltas: deltaSeen.map((c) => c.key),
      }));
      handle.onUpdate('x', { v: 0 });
      seed(handle, [
        ['a', { v: 1 }],
        ['b', { v: 2 }],
      ]);
      await expect(observed).resolves.toEqual({ rows: ['a', 'b'], deltas: ['x', 'a', 'b', 'x'] });
    });

    test('an empty server response emits settled with zero rows', () => {
      const { handle, emissions, deltas } = setup();
      expect(handle.getSnapshotMeta()).toEqual({ settled: false, hasEmitted: false });
      handle.onResult([], 'server');
      expect(emissions).toEqual([{ rows: [], settled: true }]);
      expect(deltas).toHaveLength(0);
      expect(handle.getSnapshotMeta()).toEqual({ settled: true, hasEmitted: true });
    });

    test('an empty local pre-load sets hasEmitted and does not settle', () => {
      const { handle, emissions } = setup();
      handle.onResult([], 'local');
      expect(emissions).toEqual([{ rows: [], settled: false }]);
      expect(handle.getSnapshotMeta()).toEqual({ settled: false, hasEmitted: true });
    });
  });

  describe('snapshot identity and listeners', () => {
    test('getSnapshot() is stable without a change and new on the first call after one', async () => {
      const { handle } = setup({ sort: { n: 'asc' } });
      seed(handle, [
        ['a', { n: 1 }],
        ['b', { n: 2 }],
      ]);
      const first = handle.getSnapshot();
      expect(handle.getSnapshot()).toBe(first);
      handle.onUpdate('b', { n: 0 });
      // Before the flush: a new array that already holds the update.
      const updated = handle.getSnapshot();
      expect(updated).not.toBe(first);
      expect(updated.map((r) => r._key)).toEqual(['b', 'a']);
      await nextTurn();
      expect(handle.getSnapshot()).toBe(updated);
      // The unchanged row is the same object across emissions.
      expect(updated[1]).toBe(first[0]);
    });

    test('a non-first listener added between an update and its flush gets the update at once, then the flush', async () => {
      const { handle, emissions } = setup();
      seed(handle, [['a', { v: 1 }]]);
      handle.onUpdate('b', { v: 2 });
      const late: string[][] = [];
      handle.subscribe((rows) => late.push(rows.map((r) => r._key)));
      expect(late).toEqual([['a', 'b']]);
      const before = emissions.length;
      await nextTurn();
      expect(late).toEqual([
        ['a', 'b'],
        ['a', 'b'],
      ]);
      expect(emissions).toHaveLength(before + 1);
    });

    test('a listener removed before the flush is not called; nobody registered is harmless', async () => {
      const handle = new QueryHandle<Row>(makeEngine(), 'm', {});
      const listener = jest.fn();
      const unsubscribe = handle.subscribe(listener);
      seed(handle, [['a', { v: 1 }]]);
      const calls = listener.mock.calls.length;
      handle.onUpdate('b', { v: 2 });
      unsubscribe();
      await nextTurn();
      expect(listener).toHaveBeenCalledTimes(calls);
      expect(handle.getSnapshot().map((r) => r._key)).toEqual(['a', 'b']);
    });

    test('a flush reached from a listener of a running flush delivers after it, in order', () => {
      const { handle } = setup();
      const second: string[][] = [];
      let reentered = false;
      handle.onDelta(() => {
        if (reentered) return;
        reentered = true;
        handle.onResult([{ key: 'b', value: { v: 1 } }], 'server');
      });
      handle.onDelta((changes) => second.push(changes.map((c) => `${c.type}:${c.key}`)));
      handle.onResult([{ key: 'a', value: { v: 1 } }], 'server');
      expect(second).toEqual([['add:a'], ['add:b', 'remove:a']]);
      expect(handle.getSnapshot().map((r) => r._key)).toEqual(['b']);
    });
  });
});
