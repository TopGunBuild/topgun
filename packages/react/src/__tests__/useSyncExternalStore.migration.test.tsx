/**
 * Behavioral tests for the useSyncExternalStore migration (TODO-515, folds 516).
 *
 * These are the negative-control / invariant tests the audit (DEPTH_REACT F1/F6)
 * called for:
 *   - useQuery returns cached data synchronously on first render (no
 *     {loading:true, data:[]} flash) when the handle already has a snapshot.
 *   - StrictMode double-invoke leaves a net-zero subscribe/unsubscribe balance.
 *   - A rerender that does not change identity does NOT re-subscribe.
 *   - getSnapshot is referentially stable across renders without a mutation
 *     (the property that prevents tearing / infinite render loops).
 */
import React, { StrictMode } from 'react';
import { renderHook, act, render } from '@testing-library/react';
import { TopGunProvider } from '../TopGunProvider';
import { useQuery } from '../hooks/useQuery';
import { useMap } from '../hooks/useMap';
import { TopGunClient } from '@topgunbuild/client';

// A minimal LWWMap-shaped mock (mirrors useMap.test.tsx) — importing the real
// LWWMap from @topgunbuild/core pulls in msgpackr ESM, which the package's jest
// transform does not handle. We only need subscribe + a mutation that notifies.
function createMockLWWMap() {
  const data = new Map<string, any>();
  const listeners = new Set<() => void>();
  return {
    get: (k: string) => data.get(k),
    set: (k: string, v: any) => {
      data.set(k, v);
      listeners.forEach((cb) => cb());
    },
    entries: () => Array.from(data.entries()),
    subscribe: (cb: () => void) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
}

// ---- A QueryHandle mock that exposes the new synchronous snapshot accessors.
function createSnapshotQueryHandle(initial: any[]) {
  const dataListeners = new Set<(results: any[], meta?: any) => void>();
  let snapshot = initial;
  let emitted = initial.length > 0; // already has cached data
  let subscribeCount = 0;
  let unsubscribeCount = 0;
  return {
    subscribe: jest.fn((cb: (results: any[], meta?: any) => void) => {
      subscribeCount++;
      dataListeners.add(cb);
      return () => {
        unsubscribeCount++;
        dataListeners.delete(cb);
      };
    }),
    onDelta: jest.fn(() => () => {}),
    onPaginationChange: jest.fn(() => () => {}),
    onSyncStateChange: jest.fn(() => () => {}),
    loadMore: jest.fn().mockResolvedValue(undefined),
    // Synchronous cached snapshot accessors (the migration target).
    getSnapshot: () => snapshot,
    getSnapshotMeta: () => ({ settled: emitted, hasEmitted: emitted }),
    // Test helpers
    _emit: (next: any[]) => {
      snapshot = next;
      emitted = true;
      for (const cb of dataListeners) cb(next, { settled: true });
    },
    _counts: () => ({ subscribeCount, unsubscribeCount }),
  };
}

describe('useSyncExternalStore migration — useQuery synchronous snapshot', () => {
  it('returns cached results synchronously on first render with loading:false (no loading flash)', () => {
    const handle = createSnapshotQueryHandle([{ _key: 'a', title: 'cached' }]);
    const client = {
      query: jest.fn().mockReturnValue(handle),
    } as unknown as TopGunClient;

    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <TopGunProvider client={client}>{children}</TopGunProvider>
    );

    // Capture every committed (loading, data) pair.
    const commits: Array<{ loading: boolean; len: number }> = [];
    renderHook(
      () => {
        const q = useQuery('todos');
        commits.push({ loading: q.loading, len: q.data.length });
        return q;
      },
      { wrapper },
    );

    // The FIRST commit must already carry the cached row with loading:false.
    // Negative control: before the migration this first commit was
    // {loading:true, len:0} (a flash), because subscribe did not deliver the
    // first listener synchronously and the snapshot was read async.
    expect(commits[0]).toEqual({ loading: false, len: 1 });
    // And there must be NO intermediate {loading:true, len:0} commit at all.
    expect(commits.some((c) => c.loading === true && c.len === 0)).toBe(false);
  });

  it('still falls back to loading:true when the handle has no cached data', () => {
    const handle = createSnapshotQueryHandle([]); // empty → not emitted
    const client = {
      query: jest.fn().mockReturnValue(handle),
    } as unknown as TopGunClient;
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <TopGunProvider client={client}>{children}</TopGunProvider>
    );

    const { result } = renderHook(() => useQuery('todos'), { wrapper });
    expect(result.current.loading).toBe(true);
    expect(result.current.data).toEqual([]);

    act(() => handle._emit([{ _key: 'a', title: 'arrived' }]));
    expect(result.current.loading).toBe(false);
    expect(result.current.data).toEqual([{ _key: 'a', title: 'arrived' }]);
  });
});

describe('useSyncExternalStore migration — subscribe-count invariants', () => {
  it('does NOT re-subscribe on a rerender that keeps the query identity', () => {
    const handle = createSnapshotQueryHandle([]);
    const client = {
      query: jest.fn().mockReturnValue(handle),
    } as unknown as TopGunClient;
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <TopGunProvider client={client}>{children}</TopGunProvider>
    );

    const { rerender } = renderHook(() => useQuery('todos'), { wrapper });
    const before = handle._counts().subscribeCount;

    rerender();
    rerender();
    rerender();

    expect(handle._counts().subscribeCount).toBe(before);
  });

  it('StrictMode double-invoke leaves a net-zero subscribe/unsubscribe balance (no leak)', () => {
    const handle = createSnapshotQueryHandle([]);
    const client = {
      query: jest.fn().mockReturnValue(handle),
    } as unknown as TopGunClient;

    function Probe() {
      useQuery('todos');
      return null;
    }

    const { unmount } = render(
      <StrictMode>
        <TopGunProvider client={client}>
          <Probe />
        </TopGunProvider>
      </StrictMode>,
    );
    unmount();

    const { subscribeCount, unsubscribeCount } = handle._counts();
    // Every subscribe must be matched by an unsubscribe after unmount.
    expect(subscribeCount).toBe(unsubscribeCount);
    expect(subscribeCount).toBeGreaterThan(0);
  });
});

describe('useSyncExternalStore migration — getSnapshot referential stability', () => {
  it('useQuery.data keeps the same array identity across rerenders without a mutation', () => {
    const handle = createSnapshotQueryHandle([{ _key: 'a' }]);
    const client = {
      query: jest.fn().mockReturnValue(handle),
    } as unknown as TopGunClient;
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <TopGunProvider client={client}>{children}</TopGunProvider>
    );

    const { result, rerender } = renderHook(() => useQuery('todos'), { wrapper });
    const first = result.current.data;
    rerender();
    rerender();
    // Same identity — this is the property that prevents useSyncExternalStore
    // from looping / tearing. A getSnapshot that built a fresh [] each call
    // would fail this and (under native USES) throw a max-depth loop.
    expect(result.current.data).toBe(first);
  });

  it('useMap returns the same stable LWWMap object and re-renders only on mutation', () => {
    const map = createMockLWWMap();
    const client = {
      getMap: jest.fn().mockReturnValue(map),
      getRecordSyncStateTracker: jest.fn(),
    } as unknown as TopGunClient;
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <TopGunProvider client={client}>{children}</TopGunProvider>
    );

    let renderCount = 0;
    const { result, rerender } = renderHook(
      () => {
        renderCount++;
        return useMap('cart');
      },
      { wrapper },
    );

    const mapRef = result.current;
    expect(mapRef).toBe(map);

    rerender();
    expect(result.current).toBe(map); // identity stable across rerenders

    const beforeMutation = renderCount;
    act(() => {
      map.set('item-1', { qty: 1 });
    });
    // A real mutation drives a re-render via the version-counter snapshot.
    expect(renderCount).toBeGreaterThan(beforeMutation);
    // The returned object is still the same mutable map.
    expect(result.current).toBe(map);
  });
});
