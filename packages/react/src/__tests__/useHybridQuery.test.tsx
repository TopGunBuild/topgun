import { renderHook, act } from '@testing-library/react';
import { useHybridQuery } from '../hooks/useHybridQuery';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock HybridQueryHandle
const mockSubscribe = jest.fn();
const mockGetFilter = jest.fn().mockReturnValue({});
const mockGetMapName = jest.fn().mockReturnValue('testMap');
const mockHasFTSPredicate = jest.fn().mockReturnValue(false);
const mockOnPaginationChange = jest.fn();

const mockHybridQueryHandle = {
  id: 'test-id',
  subscribe: mockSubscribe,
  getFilter: mockGetFilter,
  getMapName: mockGetMapName,
  hasFTSPredicate: mockHasFTSPredicate,
  onPaginationChange: mockOnPaginationChange,
};

// Mock TopGunClient
const mockHybridQuery = jest.fn().mockReturnValue(mockHybridQueryHandle);
const mockClient = {
  hybridQuery: mockHybridQuery,
} as unknown as TopGunClient;

describe('useHybridQuery', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSubscribe.mockReturnValue(() => {}); // Unsubscribe function
    mockOnPaginationChange.mockReturnValue(() => {});
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should initialize with loading state', () => {
    const { result } = renderHook(() => useHybridQuery('articles'), {
      wrapper,
    });

    expect(result.current.loading).toBe(true);
    expect(result.current.results).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it('should call hybridQuery with correct parameters', () => {
    const filter = {
      predicate: { op: 'match' as const, attribute: 'body', query: 'test' },
      sort: { _score: 'desc' as const },
      limit: 20,
    };

    renderHook(() => useHybridQuery('articles', filter), { wrapper });

    expect(mockHybridQuery).toHaveBeenCalledWith('articles', filter);
  });

  it('should update results when subscription fires', () => {
    let subscriptionCallback: (results: any[]) => void = () => {};
    mockSubscribe.mockImplementation((cb) => {
      subscriptionCallback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useHybridQuery('articles'), { wrapper });

    expect(result.current.loading).toBe(true);

    act(() => {
      subscriptionCallback([
        { value: { title: 'Test Article' }, _key: 'doc1', _score: 2.5, _matchedTerms: ['test'] },
        { value: { title: 'Another Test' }, _key: 'doc2', _score: 1.8, _matchedTerms: ['test'] },
      ]);
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.results).toHaveLength(2);
    expect(result.current.results[0]._key).toBe('doc1');
    expect(result.current.results[0]._score).toBe(2.5);
  });

  it('should handle subscription errors', () => {
    mockHybridQuery.mockImplementationOnce(() => {
      throw new Error('Hybrid query failed');
    });

    const { result } = renderHook(() => useHybridQuery('articles'), { wrapper });

    expect(result.current.error).toEqual(new Error('Hybrid query failed'));
    expect(result.current.loading).toBe(false);
  });

  it('should skip query when skip option is true', () => {
    const { result } = renderHook(
      () => useHybridQuery('articles', {}, { skip: true }),
      { wrapper }
    );

    expect(mockHybridQuery).not.toHaveBeenCalled();
    expect(result.current.loading).toBe(false);
    expect(result.current.results).toEqual([]);
  });

  it('should recreate handle when mapName changes', () => {
    const { rerender } = renderHook(
      ({ mapName }) => useHybridQuery(mapName),
      {
        wrapper,
        initialProps: { mapName: 'articles' },
      }
    );

    expect(mockHybridQuery).toHaveBeenCalledTimes(1);
    expect(mockHybridQuery).toHaveBeenLastCalledWith('articles', {});

    // Change mapName - should recreate handle
    rerender({ mapName: 'products' });

    expect(mockHybridQuery).toHaveBeenCalledTimes(2);
    expect(mockHybridQuery).toHaveBeenLastCalledWith('products', {});
  });

  it('should recreate handle when filter changes', () => {
    const { rerender } = renderHook(
      ({ filter }) => useHybridQuery('articles', filter),
      {
        wrapper,
        initialProps: { filter: { limit: 10 } },
      }
    );

    expect(mockHybridQuery).toHaveBeenCalledTimes(1);
    expect(mockHybridQuery).toHaveBeenLastCalledWith('articles', { limit: 10 });

    // Change filter - should recreate handle
    rerender({ filter: { limit: 20 } });

    expect(mockHybridQuery).toHaveBeenCalledTimes(2);
    expect(mockHybridQuery).toHaveBeenLastCalledWith('articles', { limit: 20 });
  });

  it('should maintain result order (by score)', () => {
    let subscriptionCallback: (results: any[]) => void = () => {};
    mockSubscribe.mockImplementation((cb) => {
      subscriptionCallback = cb;
      return () => {};
    });

    const { result } = renderHook(
      () => useHybridQuery('articles', { sort: { _score: 'desc' } }),
      { wrapper }
    );

    act(() => {
      // Results should be preserved in the order returned by HybridQueryHandle
      // (which is already sorted by score)
      subscriptionCallback([
        { value: { title: 'High Score' }, _key: 'high', _score: 5.0, _matchedTerms: ['test'] },
        { value: { title: 'Medium Score' }, _key: 'medium', _score: 3.0, _matchedTerms: ['test'] },
        { value: { title: 'Low Score' }, _key: 'low', _score: 1.0, _matchedTerms: ['test'] },
      ]);
    });

    expect(result.current.results[0]._score).toBe(5.0);
    expect(result.current.results[1]._score).toBe(3.0);
    expect(result.current.results[2]._score).toBe(1.0);
  });

  it('should handle hybrid query with FTS predicate', () => {
    const filter = {
      predicate: {
        op: 'and' as const,
        children: [
          { op: 'match' as const, attribute: 'body', query: 'machine learning' },
          { op: 'eq' as const, attribute: 'category', value: 'tech' },
        ],
      },
      sort: { _score: 'desc' as const },
      limit: 20,
    };

    renderHook(() => useHybridQuery('articles', filter), { wrapper });

    expect(mockHybridQuery).toHaveBeenCalledWith('articles', filter);
  });

  it('should handle empty results gracefully', () => {
    let subscriptionCallback: (results: any[]) => void = () => {};
    mockSubscribe.mockImplementation((cb) => {
      subscriptionCallback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useHybridQuery('articles'), { wrapper });

    act(() => {
      subscriptionCallback([]);
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.results).toEqual([]);
  });

  it('should cleanup on unmount', () => {
    const mockUnsubscribe = jest.fn();
    mockSubscribe.mockReturnValue(mockUnsubscribe);

    const { unmount } = renderHook(() => useHybridQuery('articles'), { wrapper });

    unmount();

    expect(mockUnsubscribe).toHaveBeenCalled();
  });

  it('should include matchedTerms in results', () => {
    let subscriptionCallback: (results: any[]) => void = () => {};
    mockSubscribe.mockImplementation((cb) => {
      subscriptionCallback = cb;
      return () => {};
    });

    const { result } = renderHook(
      () => useHybridQuery('articles', { predicate: { op: 'match' as const, attribute: 'body', query: 'test' } }),
      { wrapper }
    );

    act(() => {
      subscriptionCallback([
        { value: { title: 'Test Article' }, _key: 'doc1', _score: 2.5, _matchedTerms: ['test', 'testing'] },
      ]);
    });

    expect(result.current.results[0]._matchedTerms).toEqual(['test', 'testing']);
  });
});
