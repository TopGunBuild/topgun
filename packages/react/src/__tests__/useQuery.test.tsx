import { renderHook, act } from '@testing-library/react';
import { useQuery } from '../hooks/useQuery';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
// Imported by path: the client's index pulls in an ESM-only dependency this suite does not transform.
import { QueryError } from '../../../client/src/errors/QueryError';
import React from 'react';

// Mock TopGunClient
const mockSubscribe = jest.fn();
const mockOnDelta = jest.fn();
const mockOnPaginationChange = jest.fn();
const mockOnSyncStateChange = jest.fn();
const mockLoadMore = jest.fn();
const mockQuery = jest.fn().mockReturnValue({
  subscribe: mockSubscribe,
  onDelta: mockOnDelta,
  onPaginationChange: mockOnPaginationChange,
  onSyncStateChange: mockOnSyncStateChange,
  loadMore: mockLoadMore,
});
const mockClient = {
  query: mockQuery,
} as unknown as TopGunClient;

describe('useQuery', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSubscribe.mockReturnValue(() => {}); // Unsubscribe function
    mockOnDelta.mockReturnValue(() => {}); // Unsubscribe function for changes
    mockOnPaginationChange.mockReturnValue(() => {});
    mockOnSyncStateChange.mockReturnValue(() => {});
    mockLoadMore.mockResolvedValue(undefined);
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should initialize with loading state', () => {
    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    expect(result.current.loading).toBe(true);
    expect(result.current.data).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it('should update data when subscription fires', async () => {
    let callback: (results: any[]) => void;
    mockSubscribe.mockImplementation((cb) => {
      callback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    expect(result.current.loading).toBe(true);

    act(() => {
      if (callback) {
        callback([{ _key: 'item-1', id: '1', text: 'test' }]);
      }
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.data).toEqual([{ _key: 'item-1', id: '1', text: 'test' }]);
  });

  it('should handle subscription errors', () => {
    mockQuery.mockImplementationOnce(() => {
      throw new Error('Subscription failed');
    });

    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    expect(result.current.error).toEqual(new Error('Subscription failed'));
    expect(result.current.loading).toBe(false);
  });

  it('surfaces the QueryError of a live query with groupBy', () => {
    // client.query() throws this from the QueryHandle constructor for a live grouped filter.
    mockQuery.mockImplementationOnce((mapName: string) => {
      throw new QueryError(
        'QUERY_LIVE_AGGREGATE_UNSUPPORTED: run it as a one-shot query (queryOnce / queryOncePaged)',
        'QUERY_LIVE_AGGREGATE_UNSUPPORTED',
        mapName,
      );
    });

    const { result } = renderHook(() => useQuery('m', { groupBy: ['c'] }), { wrapper });

    expect(result.current.error).toBeInstanceOf(QueryError);
    expect((result.current.error as QueryError).code).toBe('QUERY_LIVE_AGGREGATE_UNSUPPORTED');
    expect(result.current.data).toEqual([]);
    expect(result.current.loading).toBe(false);
  });

  it('should initialize with empty changes and null lastChange', () => {
    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    expect(result.current.changes).toEqual([]);
    expect(result.current.lastChange).toBeNull();
    expect(typeof result.current.clearChanges).toBe('function');
  });

  // Locks in back-compat with the additive subscribe signature: the client
  // callback is `(results, meta?) => void`, and useQuery still subscribes with a
  // single-arg `(results) => void` callback. Delivering results without `meta`
  // (legacy path) and with `meta` (new path) must both update data, proving the
  // single-arg consumer ignores the extra argument gracefully.
  it('should receive results through the single-arg subscribe path regardless of the optional meta arg', () => {
    let callback: (results: any[], meta?: { settled: boolean }) => void;
    mockSubscribe.mockImplementation((cb) => {
      callback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    // Legacy single-arg invocation: no meta passed.
    act(() => {
      callback([{ _key: 'item-1', id: '1', text: 'legacy' }]);
    });
    expect(result.current.loading).toBe(false);
    expect(result.current.data).toEqual([{ _key: 'item-1', id: '1', text: 'legacy' }]);

    // New additive invocation: client passes a settled meta object. The single-arg
    // callback must ignore it and still apply results unchanged.
    act(() => {
      callback([{ _key: 'item-2', id: '2', text: 'with-meta' }], { settled: true });
    });
    expect(result.current.data).toEqual([{ _key: 'item-2', id: '2', text: 'with-meta' }]);
  });

  it('loadMore() delegates to handle.loadMore() when hasMore is true', async () => {
    let paginationCallback: (info: any) => void;
    mockOnPaginationChange.mockImplementation((cb) => {
      paginationCallback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    // Signal that more results are available
    act(() => {
      paginationCallback({ hasMore: true, nextCursor: 'cursor-abc', cursorStatus: 'valid' });
    });

    await act(async () => {
      await result.current.loadMore();
    });

    expect(mockLoadMore).toHaveBeenCalledTimes(1);
  });

  it('loadMore() is a no-op when hasMore is false', async () => {
    // Default paginationInfo has hasMore: false — no explicit trigger needed
    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    await act(async () => {
      await result.current.loadMore();
    });

    expect(mockLoadMore).not.toHaveBeenCalled();
  });

  it('loadMore ref is stable across re-renders when hasMore stays true', async () => {
    let paginationCallback: (info: any) => void;
    mockOnPaginationChange.mockImplementation((cb) => {
      paginationCallback = cb;
      return () => {};
    });

    let subscribeCallback: (results: any[]) => void;
    mockSubscribe.mockImplementation((cb) => {
      subscribeCallback = cb;
      return () => {};
    });

    const { result, rerender } = renderHook(() => useQuery('testMap', {}), { wrapper });

    act(() => {
      paginationCallback({ hasMore: true, nextCursor: 'cursor-abc', cursorStatus: 'valid' });
    });

    const loadMoreBefore = result.current.loadMore;

    // Trigger a re-render by delivering new results (hasMore stays true)
    act(() => {
      subscribeCallback([{ _key: 'item-1' }]);
    });
    rerender();

    const loadMoreAfter = result.current.loadMore;

    expect(Object.is(loadMoreBefore, loadMoreAfter)).toBe(true);
  });

  it('loadMore identity changes when hasMore flips false→true and the new callback delegates', async () => {
    let paginationCallback: (info: any) => void;
    mockOnPaginationChange.mockImplementation((cb) => {
      paginationCallback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    // Initial state: hasMore defaults to false, so loadMore is a no-op.
    const loadMoreBefore = result.current.loadMore;
    await act(async () => {
      await loadMoreBefore();
    });
    expect(mockLoadMore).not.toHaveBeenCalled();

    // Pagination becomes available — the callback identity must change so the
    // new closure observes hasMore: true (removing the dep would strand it as a no-op).
    act(() => {
      paginationCallback({ hasMore: true, nextCursor: 'cursor-abc', cursorStatus: 'valid' });
    });

    const loadMoreAfter = result.current.loadMore;
    expect(Object.is(loadMoreBefore, loadMoreAfter)).toBe(false);

    await act(async () => {
      await loadMoreAfter();
    });
    expect(mockLoadMore).toHaveBeenCalledTimes(1);
  });

  describe('a server error on the handle', () => {
    function handleWithError() {
      let emit: (results: any[]) => void = () => {};
      let raise: (error: QueryError | null) => void = () => {};
      const onError = jest.fn((listener: (error: QueryError | null) => void) => {
        raise = listener;
        listener(null);
        return () => {};
      });
      mockQuery.mockReturnValueOnce({
        subscribe: (cb: (results: any[]) => void) => {
          emit = cb;
          return () => {};
        },
        onDelta: mockOnDelta,
        onPaginationChange: mockOnPaginationChange,
        onSyncStateChange: mockOnSyncStateChange,
        onError,
        loadMore: mockLoadMore,
      });
      return { emit: (rows: any[]) => emit(rows), raise: (e: QueryError | null) => raise(e) };
    }

    const overflow = new QueryError(
      'QUERY_SNAPSHOT_OVERFLOW: too many concurrent writes',
      'QUERY_SNAPSHOT_OVERFLOW',
      'm',
    );

    it('surfaces the QueryError, keeps the rows and is not loading', () => {
      const handle = handleWithError();
      const { result } = renderHook(() => useQuery('m', {}), { wrapper });
      const rows = [{ _key: 'a', n: 1 }];
      act(() => handle.emit(rows));
      act(() => handle.raise(overflow));

      expect(result.current.error).toBeInstanceOf(QueryError);
      expect((result.current.error as QueryError).code).toBe('QUERY_SNAPSHOT_OVERFLOW');
      expect(result.current.data).toEqual(rows);
      expect(result.current.loading).toBe(false);

      act(() => handle.raise(null));
      expect(result.current.error).toBeNull();
      expect(result.current.data).toEqual(rows);
    });

    it('is not loading when the error comes before any rows', () => {
      const handle = handleWithError();
      const { result } = renderHook(() => useQuery('m', {}), { wrapper });
      expect(result.current.loading).toBe(true);
      act(() => handle.raise(overflow));
      expect(result.current.error).toBe(overflow);
      expect(result.current.loading).toBe(false);
    });
  });
});
