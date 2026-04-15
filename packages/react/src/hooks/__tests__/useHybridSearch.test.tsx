import { renderHook, act, waitFor } from '@testing-library/react';
import { useHybridSearch } from '../useHybridSearch';
import { TopGunProvider } from '../../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock TopGunClient with hybridSearch method
const mockHybridSearch = jest.fn();
const mockClient = {
  hybridSearch: mockHybridSearch,
} as unknown as TopGunClient;

describe('useHybridSearch', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  // ============================================
  // AC #8: null and empty queryText skip execution
  // ============================================

  it('should return empty results when queryText is null (AC #8)', () => {
    const { result } = renderHook(() => useHybridSearch('docs', null), { wrapper });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockHybridSearch).not.toHaveBeenCalled();
  });

  it('should return empty results when queryText is empty string (AC #8)', () => {
    const { result } = renderHook(() => useHybridSearch('docs', ''), { wrapper });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockHybridSearch).not.toHaveBeenCalled();
  });

  it('should return empty results when enabled is false', () => {
    const { result } = renderHook(
      () => useHybridSearch('docs', 'query', { enabled: false }),
      { wrapper }
    );

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockHybridSearch).not.toHaveBeenCalled();
  });

  // ============================================
  // AC #7: loading transitions and results
  // ============================================

  it('should transition through loading:true then loading:false with results (AC #7)', async () => {
    const mockResults = [
      { key: 'doc-1', score: 0.95, methodScores: { fullText: 0.9 } },
      { key: 'doc-2', score: 0.87, methodScores: { fullText: 0.8 } },
    ];
    mockHybridSearch.mockResolvedValue(mockResults);

    const { result } = renderHook(
      () => useHybridSearch('docs', 'machine learning', { methods: ['fullText'], k: 10 }),
      { wrapper }
    );

    // Initially loading
    expect(result.current.loading).toBe(true);

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.results).toEqual(mockResults);
    expect(result.current.error).toBeNull();
    expect(mockHybridSearch).toHaveBeenCalledWith('docs', 'machine learning', {
      methods: ['fullText'],
      k: 10,
    });
  });

  it('should set error state on rejection', async () => {
    mockHybridSearch.mockRejectedValue(new Error('Map not found'));

    const { result } = renderHook(
      () => useHybridSearch('docs', 'test query'),
      { wrapper }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toEqual(new Error('Map not found'));
    expect(result.current.results).toEqual([]);
  });

  // ============================================
  // AC #9: Float32Array element-wise dependency key
  // ============================================

  it('should not re-fetch when Float32Array reference changes but content is the same (AC #9)', async () => {
    mockHybridSearch.mockResolvedValue([{ key: 'doc-1', score: 0.9, methodScores: {} }]);

    const { result, rerender } = renderHook(
      ({ vector }) =>
        useHybridSearch('docs', 'query', {
          methods: ['semantic'],
          queryVector: vector,
        }),
      {
        wrapper,
        initialProps: { vector: new Float32Array([0.1, 0.2]) as Float32Array | number[] | undefined },
      }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(mockHybridSearch).toHaveBeenCalledTimes(1);

    // Re-render with a new Float32Array reference containing the same values
    rerender({ vector: new Float32Array([0.1, 0.2]) });

    // Should NOT trigger a second call since content is identical (element-wise key)
    expect(mockHybridSearch).toHaveBeenCalledTimes(1);
  });

  it('should re-fire when vector content changes', async () => {
    mockHybridSearch.mockResolvedValue([]);

    const { result, rerender } = renderHook(
      ({ vector }) =>
        useHybridSearch('docs', 'query', {
          queryVector: vector,
        }),
      {
        wrapper,
        initialProps: { vector: [0.1, 0.2] as Float32Array | number[] | undefined },
      }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(mockHybridSearch).toHaveBeenCalledTimes(1);

    // Change the vector content
    rerender({ vector: [0.3, 0.4] });

    await waitFor(() => {
      expect(mockHybridSearch).toHaveBeenCalledTimes(2);
    });
  });

  it('should re-fire when methods array changes', async () => {
    mockHybridSearch.mockResolvedValue([]);

    const { result, rerender } = renderHook(
      ({ methods }) =>
        useHybridSearch('docs', 'query', { methods }),
      {
        wrapper,
        initialProps: { methods: ['fullText'] as ('exact' | 'fullText' | 'semantic')[] },
      }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(mockHybridSearch).toHaveBeenCalledTimes(1);

    rerender({ methods: ['fullText', 'semantic'] });

    await waitFor(() => {
      expect(mockHybridSearch).toHaveBeenCalledTimes(2);
    });
  });

  it('should re-fire when queryText changes', async () => {
    mockHybridSearch.mockResolvedValue([]);

    const { result, rerender } = renderHook(
      ({ queryText }) => useHybridSearch('docs', queryText),
      {
        wrapper,
        initialProps: { queryText: 'first query' },
      }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(mockHybridSearch).toHaveBeenCalledTimes(1);

    rerender({ queryText: 'second query' });

    await waitFor(() => {
      expect(mockHybridSearch).toHaveBeenCalledTimes(2);
    });
  });

  it('should discard stale responses when queryText changes', async () => {
    let resolveFirst: (value: any) => void;
    const firstPromise = new Promise((resolve) => {
      resolveFirst = resolve;
    });

    const secondResults = [{ key: 'doc-new', score: 0.99, methodScores: {} }];

    mockHybridSearch
      .mockReturnValueOnce(firstPromise)
      .mockResolvedValueOnce(secondResults);

    const { result, rerender } = renderHook(
      ({ queryText }) => useHybridSearch('docs', queryText),
      {
        wrapper,
        initialProps: { queryText: 'first' },
      }
    );

    // Change query before first resolves — first response becomes stale
    rerender({ queryText: 'second' });

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    // Resolve the stale first request after the second has completed
    await act(async () => {
      resolveFirst!([{ key: 'doc-stale', score: 0.5, methodScores: {} }]);
    });

    // Should show the second result, not the stale first
    expect(result.current.results).toEqual(secondResults);
  });

  it('should reset state when queryText becomes null', async () => {
    mockHybridSearch.mockResolvedValue([{ key: 'doc-1', score: 0.9, methodScores: {} }]);

    const { result, rerender } = renderHook(
      ({ queryText }) => useHybridSearch('docs', queryText),
      {
        wrapper,
        initialProps: { queryText: 'active query' as string | null },
      }
    );

    await waitFor(() => {
      expect(result.current.results).toHaveLength(1);
    });

    rerender({ queryText: null });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('should reset state when queryText becomes empty string', async () => {
    mockHybridSearch.mockResolvedValue([{ key: 'doc-1', score: 0.9, methodScores: {} }]);

    const { result, rerender } = renderHook(
      ({ queryText }) => useHybridSearch('docs', queryText),
      {
        wrapper,
        initialProps: { queryText: 'active query' as string | null },
      }
    );

    await waitFor(() => {
      expect(result.current.results).toHaveLength(1);
    });

    rerender({ queryText: '' });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });
});
