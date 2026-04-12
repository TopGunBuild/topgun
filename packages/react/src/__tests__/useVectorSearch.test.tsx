import { renderHook, act, waitFor } from '@testing-library/react';
import { useVectorSearch } from '../hooks/useVectorSearch';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock TopGunClient with vectorSearch method
const mockVectorSearch = jest.fn();
const mockClient = {
  vectorSearch: mockVectorSearch,
} as unknown as TopGunClient;

describe('useVectorSearch', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should return empty results when query is null', () => {
    const { result } = renderHook(() => useVectorSearch('notes', null), { wrapper });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockVectorSearch).not.toHaveBeenCalled();
  });

  it('should return empty results when enabled is false', () => {
    const { result } = renderHook(
      () => useVectorSearch('notes', new Float32Array([0.1, 0.2]), { enabled: false }),
      { wrapper }
    );

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockVectorSearch).not.toHaveBeenCalled();
  });

  it('should execute search and return results', async () => {
    const mockResults = [
      { key: 'doc-1', score: 0.95 },
      { key: 'doc-2', score: 0.87 },
    ];
    mockVectorSearch.mockResolvedValue(mockResults);

    const { result } = renderHook(
      () => useVectorSearch('notes', new Float32Array([0.1, 0.2, 0.3]), { k: 5 }),
      { wrapper }
    );

    // Initially loading
    expect(result.current.loading).toBe(true);

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.results).toEqual(mockResults);
    expect(result.current.error).toBeNull();
    expect(mockVectorSearch).toHaveBeenCalledWith(
      'notes',
      expect.any(Float32Array),
      { k: 5 }
    );
  });

  it('should set error state on rejection', async () => {
    mockVectorSearch.mockRejectedValue(new Error('Index not found'));

    const { result } = renderHook(
      () => useVectorSearch('notes', [0.1, 0.2]),
      { wrapper }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toEqual(new Error('Index not found'));
    expect(result.current.results).toEqual([]);
  });

  it('should not re-fire when Float32Array reference changes but content is the same', async () => {
    mockVectorSearch.mockResolvedValue([{ key: 'doc-1', score: 0.9 }]);

    const { result, rerender } = renderHook(
      ({ query }) => useVectorSearch('notes', query),
      {
        wrapper,
        initialProps: { query: new Float32Array([0.1, 0.2]) as Float32Array | number[] | null },
      }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(mockVectorSearch).toHaveBeenCalledTimes(1);

    // Re-render with a new Float32Array reference containing the same values
    rerender({ query: new Float32Array([0.1, 0.2]) });

    // Should NOT trigger a second call since content is identical
    expect(mockVectorSearch).toHaveBeenCalledTimes(1);
  });

  it('should re-fire when vector content changes', async () => {
    mockVectorSearch.mockResolvedValue([]);

    const { result, rerender } = renderHook(
      ({ query }) => useVectorSearch('notes', query),
      {
        wrapper,
        initialProps: { query: [0.1, 0.2] as Float32Array | number[] | null },
      }
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(mockVectorSearch).toHaveBeenCalledTimes(1);

    // Change the vector content
    rerender({ query: [0.3, 0.4] });

    await waitFor(() => {
      expect(mockVectorSearch).toHaveBeenCalledTimes(2);
    });
  });

  it('should discard stale responses when query changes', async () => {
    let resolveFirst: (value: any) => void;
    const firstPromise = new Promise((resolve) => {
      resolveFirst = resolve;
    });

    const secondResults = [{ key: 'doc-new', score: 0.99 }];

    mockVectorSearch
      .mockReturnValueOnce(firstPromise)
      .mockResolvedValueOnce(secondResults);

    const { result, rerender } = renderHook(
      ({ query }) => useVectorSearch('notes', query),
      {
        wrapper,
        initialProps: { query: [0.1] as Float32Array | number[] | null },
      }
    );

    // Change query before first resolves — first response becomes stale
    rerender({ query: [0.2] });

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    // Now resolve the stale first request
    await act(async () => {
      resolveFirst!([{ key: 'doc-stale', score: 0.5 }]);
    });

    // Should show the second result, not the stale first
    expect(result.current.results).toEqual(secondResults);
  });

  it('should reset state when query becomes null', async () => {
    mockVectorSearch.mockResolvedValue([{ key: 'doc-1', score: 0.9 }]);

    const { result, rerender } = renderHook(
      ({ query }) => useVectorSearch('notes', query),
      {
        wrapper,
        initialProps: { query: [0.1, 0.2] as Float32Array | number[] | null },
      }
    );

    await waitFor(() => {
      expect(result.current.results).toHaveLength(1);
    });

    // Set query to null
    rerender({ query: null });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });
});
