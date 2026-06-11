import { renderHook, act } from '@testing-library/react';
import { useQuery } from '../hooks/useQuery';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
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
});
