/**
 * QueryHandle live-window top-N clamp tests.
 *
 * A live `limit:N` subscription must clamp its rendered result set to N rows,
 * agreeing with the server's authoritative top-N window. The clamp is a
 * render-time slice of the sorted projection — it must NOT evict rows from the
 * in-memory `currentResults`, so it composes with the server's displacement
 * LEAVE retractions without double-dropping to N-1. The clamp is disengaged
 * once the handle enters page-accumulation mode (loadMore), so intentionally
 * accumulated multi-page results are never truncated to one page.
 *
 * The C-LIVEQ reproduction (client SDK audit negative control) is promoted here
 * with its assertion inverted: a third matching live ENTER stays clamped at 2,
 * not 3.
 */

import { QueryHandle } from '../QueryHandle';
import { SyncEngine } from '../SyncEngine';

// Mock SyncEngine — the QueryHandle clamp logic is pure and only needs the
// subscribe/unsubscribe/runLocalQuery surface plus a sync-state tracker stub.
const makeMockEngine = () =>
  ({
    subscribeToQuery: jest.fn(),
    unsubscribeFromQuery: jest.fn(),
    runLocalQuery: jest.fn().mockResolvedValue([]),
    getRecordSyncStateTracker: () => ({
      onChange: () => () => {},
      get: () => 'synced',
    }),
  }) as unknown as SyncEngine;

describe('QueryHandle live-window top-N clamp', () => {
  // AC1 — clamp boundary conditions.
  describe('AC1: getSortedResults clamps to a positive-integer limit only', () => {
    const seed = (handle: QueryHandle<{ n: number }>): any[] => {
      let last: any[] = [];
      handle.subscribe((r) => (last = r));
      handle.onResult(
        [
          { key: 'a', value: { n: 1 } },
          { key: 'b', value: { n: 2 } },
          { key: 'c', value: { n: 3 } },
          { key: 'd', value: { n: 4 } },
        ],
        'server',
      );
      return last;
    };

    test('positive-integer limit slices to N (head of sort order)', () => {
      const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
        limit: 2,
        sort: { n: 'asc' },
      });
      const last = seed(handle);
      expect(last).toHaveLength(2);
      expect(last.map((r) => r._key)).toEqual(['a', 'b']);
    });

    test('unset limit returns the full set', () => {
      const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
        sort: { n: 'asc' },
      });
      expect(seed(handle)).toHaveLength(4);
    });

    test('limit:0 returns the full set (no clamp)', () => {
      const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
        limit: 0,
        sort: { n: 'asc' },
      });
      expect(seed(handle)).toHaveLength(4);
    });

    test('negative limit returns the full set (no clamp)', () => {
      const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
        limit: -1,
        sort: { n: 'asc' },
      });
      expect(seed(handle)).toHaveLength(4);
    });

    test('non-finite limit returns the full set (no clamp)', () => {
      const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
        limit: Number.POSITIVE_INFINITY,
        sort: { n: 'asc' },
      });
      expect(seed(handle)).toHaveLength(4);
    });

    test('fractional (non-integer) limit returns the full set (no clamp)', () => {
      const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
        limit: 2.5,
        sort: { n: 'asc' },
      });
      expect(seed(handle)).toHaveLength(4);
    });
  });

  // AC2 — inverted C-LIVEQ: a third live ENTER stays clamped at 2.
  test('AC2: live limit:2 holding a,b stays clamped to [a,b] after onUpdate(c)', () => {
    const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
      limit: 2,
      sort: { n: 'asc' },
    });
    let last: any[] = [];
    handle.subscribe((r) => (last = r));

    // Server delivers its authoritative 2-row window.
    handle.onResult(
      [
        { key: 'a', value: { n: 1 } },
        { key: 'b', value: { n: 2 } },
      ],
      'server',
    );
    expect(last).toHaveLength(2);

    // A third matching live ENTER arrives. Without the clamp the rendered set
    // would grow to 3 (the audit's negative control). With the clamp it stays
    // exactly the two lowest-n rows; c (highest n) is excluded under asc sort.
    handle.onUpdate('c', { n: 3 });
    expect(last).toHaveLength(2);
    expect(last.map((r) => r._key)).toEqual(['a', 'b']);
    expect(last.map((r) => r._key)).not.toContain('c');
  });

  // AC3 — an ENTER beyond the window plus a displacement LEAVE nets to N, not N-1.
  test('AC3: ENTER of a new lowest then displacement LEAVE nets to exactly 2 (no double-drop)', () => {
    const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
      limit: 2,
      sort: { n: 'asc' },
    });
    let last: any[] = [];
    handle.subscribe((r) => (last = r));

    handle.onResult(
      [
        { key: 'a', value: { n: 1 } },
        { key: 'b', value: { n: 2 } },
      ],
      'server',
    );

    // New lowest row enters — in-memory set is now {c:0, a:1, b:2}; the rendered
    // window clamps to [c, a]. b is displaced but still held in currentResults.
    handle.onUpdate('c', { n: 0 });
    expect(last.map((r) => r._key)).toEqual(['c', 'a']);

    // Server then sends the displacement LEAVE for the now-out-of-window b.
    // Because the clamp was a pure slice (not an eviction), this nets to exactly
    // 2 rows — NOT 1. A destructive clamp would have already dropped b and this
    // LEAVE would over-prune to a single row.
    handle.onUpdate('b', null);
    expect(last).toHaveLength(2);
    expect(last.map((r) => r._key)).toEqual(['c', 'a']);
  });

  // AC4 — behavioral proof the clamp does not evict from currentResults: a
  // later in-window LEAVE promotes the previously-excluded row back into view.
  test('AC4: excluded row is retained in memory and promoted back after an in-window LEAVE', () => {
    const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
      limit: 2,
      sort: { n: 'asc' },
    });
    let last: any[] = [];
    handle.subscribe((r) => (last = r));

    handle.onResult(
      [
        { key: 'a', value: { n: 1 } },
        { key: 'b', value: { n: 2 } },
      ],
      'server',
    );

    // c is excluded by the clamp (highest n) but must remain in currentResults.
    handle.onUpdate('c', { n: 3 });
    expect(last.map((r) => r._key)).toEqual(['a', 'b']);

    // A LEAVE of the in-window row a frees a slot. If c had been evicted the
    // rendered set would now be just [b]; instead c is promoted into the window,
    // proving the clamp never removed it from currentResults.
    handle.onUpdate('a', null);
    expect(last).toHaveLength(2);
    expect(last.map((r) => r._key)).toEqual(['b', 'c']);
  });

  // AC6 — no-limit regression: a query without a limit returns all rows.
  test('AC6: a query with no limit returns all rows (no regression)', () => {
    const handle = new QueryHandle<{ n: number }>(makeMockEngine(), 'm', {
      sort: { n: 'asc' },
    });
    let last: any[] = [];
    handle.subscribe((r) => (last = r));

    handle.onResult(
      [
        { key: 'a', value: { n: 1 } },
        { key: 'b', value: { n: 2 } },
        { key: 'c', value: { n: 3 } },
      ],
      'server',
    );
    expect(last).toHaveLength(3);
    expect(last.map((r) => r._key)).toEqual(['a', 'b', 'c']);

    // Live ENTERs keep growing the set — no clamp engages.
    handle.onUpdate('d', { n: 4 });
    expect(last).toHaveLength(4);
  });

  // AC8 — after loadMore accumulates 2 pages of a limit:N live query, all
  // accumulated rows are returned (> N); the _paginated flag disengages the clamp.
  test('AC8: loadMore accumulation returns all rows (> limit), not clamped to one page', async () => {
    const engine = makeMockEngine();

    // The temp page-handle that loadMore spins up calls subscribeToQuery and
    // runLocalQuery; we deliver its page-2 rows via the server path and settle it.
    (engine.subscribeToQuery as jest.Mock).mockImplementation(
      (temp: QueryHandle<{ n: number }>) => {
        temp.onResult(
          [
            { key: 'c', value: { n: 3 } },
            { key: 'd', value: { n: 4 } },
          ],
          'server',
        );
      },
    );

    const handle = new QueryHandle<{ n: number }>(engine, 'm', {
      limit: 2,
      sort: { n: 'asc' },
    });
    let last: any[] = [];
    handle.subscribe((r) => (last = r));

    // Page 1 (the live window) holds 2 rows and is clamped.
    handle.onResult(
      [
        { key: 'a', value: { n: 1 } },
        { key: 'b', value: { n: 2 } },
      ],
      'server',
    );
    expect(last).toHaveLength(2);

    // Advance pagination state so loadMore issues a request.
    handle.updatePaginationInfo({ nextCursor: 'cursor-2', hasMore: true, cursorStatus: 'valid' });

    await handle.loadMore();

    // After accumulating page 2, all 4 rows are visible — the clamp is disengaged
    // because the handle is now in page-accumulation mode (_paginated === true).
    expect(last.length).toBeGreaterThan(2);
    expect(last).toHaveLength(4);
    expect(last.map((r) => r._key)).toEqual(['a', 'b', 'c', 'd']);
  });
});
