import { renderHook, act } from '@testing-library/react';
import { useSearch } from '../hooks/useSearch';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock SearchHandle
const mockSubscribe = jest.fn();
const mockDispose = jest.fn();
const mockSetQuery = jest.fn();
const mockSetOptions = jest.fn();

const mockSearchHandle = {
  subscribe: mockSubscribe,
  dispose: mockDispose,
  setQuery: mockSetQuery,
  setOptions: mockSetOptions,
  getResults: jest.fn().mockReturnValue([]),
  mapName: 'testMap',
  query: 'test query',
};

// Mock TopGunClient
const mockSearchSubscribe = jest.fn().mockReturnValue(mockSearchHandle);
const mockClient = {
  searchSubscribe: mockSearchSubscribe,
} as unknown as TopGunClient;

describe('useSearch', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.useFakeTimers();
    mockSubscribe.mockReturnValue(() => {}); // Unsubscribe function
  });

  afterEach(() => {
    jest.useRealTimers();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should initialize with loading state', () => {
    const { result } = renderHook(() => useSearch('articles', 'machine learning'), {
      wrapper,
    });

    expect(result.current.loading).toBe(true);
    expect(result.current.results).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it('should call searchSubscribe with correct parameters', () => {
    renderHook(() => useSearch('articles', 'machine learning', { limit: 20, boost: { title: 2.0 } }), {
      wrapper,
    });

    expect(mockSearchSubscribe).toHaveBeenCalledWith('articles', 'machine learning', {
      limit: 20,
      boost: { title: 2.0 },
    });
  });

  it('should update results when subscription fires', () => {
    let subscriptionCallback: (results: any[]) => void = () => {};
    mockSubscribe.mockImplementation((cb) => {
      subscriptionCallback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useSearch('articles', 'test'), { wrapper });

    expect(result.current.loading).toBe(true);

    act(() => {
      subscriptionCallback([
        { key: 'doc1', value: { title: 'Test Article' }, score: 2.5, matchedTerms: ['test'] },
        { key: 'doc2', value: { title: 'Another Test' }, score: 1.8, matchedTerms: ['test'] },
      ]);
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.results).toHaveLength(2);
    expect(result.current.results[0].key).toBe('doc1');
    expect(result.current.results[0].score).toBe(2.5);
  });

  it('should handle subscription errors', () => {
    mockSearchSubscribe.mockImplementationOnce(() => {
      throw new Error('Search subscription failed');
    });

    const { result } = renderHook(() => useSearch('articles', 'test'), { wrapper });

    expect(result.current.error).toEqual(new Error('Search subscription failed'));
    expect(result.current.loading).toBe(false);
  });

  it('should dispose handle on unmount', () => {
    const { unmount } = renderHook(() => useSearch('articles', 'test'), { wrapper });

    unmount();

    expect(mockDispose).toHaveBeenCalled();
  });

  it('should handle empty query gracefully', () => {
    const { result } = renderHook(() => useSearch('articles', ''), { wrapper });

    expect(result.current.loading).toBe(false);
    expect(result.current.results).toEqual([]);
    expect(mockSearchSubscribe).not.toHaveBeenCalled();
  });

  it('should handle whitespace-only query gracefully', () => {
    const { result } = renderHook(() => useSearch('articles', '   '), { wrapper });

    expect(result.current.loading).toBe(false);
    expect(result.current.results).toEqual([]);
    expect(mockSearchSubscribe).not.toHaveBeenCalled();
  });

  it('should use setQuery() when query changes (optimization)', () => {
    const { rerender } = renderHook(
      ({ query }) => useSearch('articles', query),
      {
        wrapper,
        initialProps: { query: 'first query' },
      }
    );

    // Initial subscription created
    expect(mockSearchSubscribe).toHaveBeenCalledTimes(1);
    expect(mockSearchSubscribe).toHaveBeenLastCalledWith('articles', 'first query', {});

    // Change query - should use setQuery() instead of creating new handle
    rerender({ query: 'second query' });

    // searchSubscribe should NOT be called again - setQuery() should be used instead
    expect(mockSearchSubscribe).toHaveBeenCalledTimes(1);
    expect(mockSetQuery).toHaveBeenCalledWith('second query');
  });

  it('should use setOptions() when options change (optimization)', () => {
    const { rerender } = renderHook(
      ({ limit }) => useSearch('articles', 'test', { limit }),
      {
        wrapper,
        initialProps: { limit: 10 },
      }
    );

    // Initial subscription created
    expect(mockSearchSubscribe).toHaveBeenCalledTimes(1);
    expect(mockSearchSubscribe).toHaveBeenCalledWith('articles', 'test', { limit: 10 });

    // Change options - should use setOptions() instead of creating new handle
    rerender({ limit: 20 });

    // searchSubscribe should NOT be called again - setOptions() should be used instead
    expect(mockSearchSubscribe).toHaveBeenCalledTimes(1);
    expect(mockSetOptions).toHaveBeenCalledWith({ limit: 20 });
  });

  it('should recreate handle when mapName changes', () => {
    const { rerender } = renderHook(
      ({ mapName }) => useSearch(mapName, 'test'),
      {
        wrapper,
        initialProps: { mapName: 'articles' },
      }
    );

    expect(mockSearchSubscribe).toHaveBeenCalledTimes(1);
    expect(mockSearchSubscribe).toHaveBeenLastCalledWith('articles', 'test', {});

    // Change mapName - should recreate handle
    rerender({ mapName: 'products' });

    expect(mockDispose).toHaveBeenCalled();
    expect(mockSearchSubscribe).toHaveBeenCalledTimes(2);
    expect(mockSearchSubscribe).toHaveBeenLastCalledWith('products', 'test', {});
  });

  describe('debounce', () => {
    it('should debounce query changes when debounceMs is set', () => {
      const { rerender } = renderHook(
        ({ query }) => useSearch('articles', query, { debounceMs: 300 }),
        {
          wrapper,
          initialProps: { query: 'initial' },
        }
      );

      // Initial query is not debounced - search starts immediately on mount
      expect(mockSearchSubscribe).toHaveBeenCalledTimes(1);
      expect(mockSearchSubscribe).toHaveBeenLastCalledWith('articles', 'initial', {});

      // Type rapidly - these changes should be debounced
      rerender({ query: 'in' });
      rerender({ query: 'inp' });
      rerender({ query: 'inpu' });
      rerender({ query: 'input' });

      // Should not have called setQuery yet (debouncing)
      expect(mockSetQuery).not.toHaveBeenCalled();

      // Fast-forward past debounce
      act(() => {
        jest.advanceTimersByTime(300);
      });

      // Now setQuery should have been called with the final value
      expect(mockSetQuery).toHaveBeenCalledWith('input');
    });

    it('should not debounce when debounceMs is not set', () => {
      const { rerender } = renderHook(
        ({ query }) => useSearch('articles', query),
        {
          wrapper,
          initialProps: { query: 'first' },
        }
      );

      expect(mockSearchSubscribe).toHaveBeenCalledWith('articles', 'first', {});

      rerender({ query: 'second' });

      // Should immediately call setQuery (no debounce)
      expect(mockSetQuery).toHaveBeenCalledWith('second');
    });

    it('should not debounce when debounceMs is 0', () => {
      const { rerender } = renderHook(
        ({ query }) => useSearch('articles', query, { debounceMs: 0 }),
        {
          wrapper,
          initialProps: { query: 'first' },
        }
      );

      expect(mockSearchSubscribe).toHaveBeenCalledWith('articles', 'first', {});

      rerender({ query: 'second' });

      // Should immediately call setQuery (no debounce)
      expect(mockSetQuery).toHaveBeenCalledWith('second');
    });
  });

  it('should pass search options to searchSubscribe', () => {
    renderHook(
      () =>
        useSearch('products', 'laptop', {
          limit: 50,
          minScore: 0.5,
          boost: { name: 2.0, description: 1.0 },
        }),
      { wrapper }
    );

    expect(mockSearchSubscribe).toHaveBeenCalledWith('products', 'laptop', {
      limit: 50,
      minScore: 0.5,
      boost: { name: 2.0, description: 1.0 },
    });
  });

  it('should not pass debounceMs to searchSubscribe', () => {
    renderHook(
      () =>
        useSearch('products', 'laptop', {
          limit: 10,
          debounceMs: 300,
        }),
      { wrapper }
    );

    // debounceMs should not be passed to searchSubscribe
    expect(mockSearchSubscribe).toHaveBeenCalledWith('products', 'laptop', {
      limit: 10,
    });
  });

  it('should clean up debounce timeout on unmount', () => {
    const { rerender, unmount } = renderHook(
      ({ query }) => useSearch('articles', query, { debounceMs: 300 }),
      { wrapper, initialProps: { query: 'initial' } }
    );

    // Initial search fires immediately
    expect(mockSearchSubscribe).toHaveBeenCalledTimes(1);

    // Start typing - triggers debounce
    rerender({ query: 'new query' });

    // Unmount before debounce fires
    unmount();

    // This should not cause any errors
    act(() => {
      jest.advanceTimersByTime(300);
    });

    // setQuery should not have been called after unmount
    expect(mockSetQuery).not.toHaveBeenCalled();
  });

  it('should maintain result order (by score)', () => {
    let subscriptionCallback: (results: any[]) => void = () => {};
    mockSubscribe.mockImplementation((cb) => {
      subscriptionCallback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useSearch('articles', 'test'), { wrapper });

    act(() => {
      // Results should be preserved in the order returned by SearchHandle
      // (which is already sorted by score)
      subscriptionCallback([
        { key: 'high', value: { title: 'High Score' }, score: 5.0, matchedTerms: ['test'] },
        { key: 'medium', value: { title: 'Medium Score' }, score: 3.0, matchedTerms: ['test'] },
        { key: 'low', value: { title: 'Low Score' }, score: 1.0, matchedTerms: ['test'] },
      ]);
    });

    expect(result.current.results[0].score).toBe(5.0);
    expect(result.current.results[1].score).toBe(3.0);
    expect(result.current.results[2].score).toBe(1.0);
  });
});
