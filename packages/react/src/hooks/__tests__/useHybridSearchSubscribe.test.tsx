import { renderHook, act, waitFor } from '@testing-library/react';
import { useHybridSearchSubscribe } from '../useHybridSearchSubscribe';
import { TopGunProvider } from '../../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock HybridSearchHandle
const mockDispose = jest.fn();
const mockSubscribe = jest.fn();
const mockSetQuery = jest.fn();
const mockSetOptions = jest.fn();

const createMockHandle = (
  subscribeCb?: (callback: (results: any[]) => void) => () => void
) => ({
  dispose: mockDispose,
  subscribe: subscribeCb ?? mockSubscribe,
  setQuery: mockSetQuery,
  setOptions: mockSetOptions,
  mapName: 'docs',
  query: 'query',
  size: 0,
  isDisposed: () => false,
  getResults: () => [],
});

const mockHybridSearchSubscribe = jest.fn();
const mockClient = {
  hybridSearchSubscribe: mockHybridSearchSubscribe,
} as unknown as TopGunClient;

describe('useHybridSearchSubscribe', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSubscribe.mockReturnValue(() => {});
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  // ============================================
  // AC #10 — loading transitions and live delta updates
  // ============================================

  it('AC #10: transitions loading:true → false when initial results arrive, then reflects HYBRID_SEARCH_UPDATE deltas', async () => {
    let capturedCallback: ((results: any[]) => void) | null = null;

    const handle = createMockHandle((cb) => {
      capturedCallback = cb;
      return () => {};
    });

    mockHybridSearchSubscribe.mockReturnValue(handle);

    const { result } = renderHook(
      () => useHybridSearchSubscribe('docs', 'query', { methods: ['fullText'] }),
      { wrapper }
    );

    // Should be loading initially
    expect(result.current.loading).toBe(true);
    expect(result.current.results).toEqual([]);

    // Simulate HYBRID_SEARCH_RESP (initial snapshot arrives via handle.subscribe callback)
    const initialResults = [
      { key: 'doc1', score: 0.9, methodScores: { fullText: 0.9 }, value: { title: 'First' } },
    ];
    act(() => {
      capturedCallback!(initialResults);
    });

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.results).toEqual(initialResults);
    expect(result.current.error).toBeNull();

    // Simulate a delta update
    const updatedResults = [
      ...initialResults,
      { key: 'doc2', score: 0.7, methodScores: { fullText: 0.7 }, value: { title: 'Second' } },
    ];
    act(() => {
      capturedCallback!(updatedResults);
    });

    expect(result.current.results).toEqual(updatedResults);
  });

  // ============================================
  // AC #11 — empty/whitespace queryText and enabled:false skip subscription
  // ============================================

  it('AC #11a: returns { results:[], loading:false, error:null } and does NOT subscribe when queryText is empty', () => {
    const { result } = renderHook(
      () => useHybridSearchSubscribe('docs', ''),
      { wrapper }
    );

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockHybridSearchSubscribe).not.toHaveBeenCalled();
  });

  it('AC #11b: returns { results:[], loading:false, error:null } and does NOT subscribe when queryText is whitespace-only', () => {
    const { result } = renderHook(
      () => useHybridSearchSubscribe('docs', '   '),
      { wrapper }
    );

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockHybridSearchSubscribe).not.toHaveBeenCalled();
  });

  it('AC #11c: returns { results:[], loading:false, error:null } and does NOT subscribe when enabled is false', () => {
    const { result } = renderHook(
      () => useHybridSearchSubscribe('docs', 'query', { enabled: false }),
      { wrapper }
    );

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockHybridSearchSubscribe).not.toHaveBeenCalled();
  });

  // ============================================
  // AC #12 — new Float32Array with identical content does NOT cause re-subscription
  // ============================================

  it('AC #12: new Float32Array reference with identical content does NOT re-subscribe (element-wise dep key)', async () => {
    let capturedCallback: ((results: any[]) => void) | null = null;
    const handle = createMockHandle((cb) => {
      capturedCallback = cb;
      return () => {};
    });
    mockHybridSearchSubscribe.mockReturnValue(handle);

    const { result, rerender } = renderHook(
      ({ vector }: { vector: Float32Array | number[] }) =>
        useHybridSearchSubscribe('docs', 'query', {
          methods: ['semantic'],
          queryVector: vector,
        }),
      {
        wrapper,
        initialProps: { vector: new Float32Array([0.1, 0.2]) },
      }
    );

    act(() => {
      capturedCallback!([]);
    });
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(mockHybridSearchSubscribe).toHaveBeenCalledTimes(1);

    // Re-render with a NEW reference but same values
    rerender({ vector: new Float32Array([0.1, 0.2]) });

    // Element-wise key is identical → no re-subscription
    expect(mockHybridSearchSubscribe).toHaveBeenCalledTimes(1);
  });

  it('re-subscribes when Float32Array content changes', async () => {
    let capturedCallback: ((results: any[]) => void) | null = null;
    const handle = createMockHandle((cb) => {
      capturedCallback = cb;
      return () => {};
    });
    mockHybridSearchSubscribe.mockReturnValue(handle);

    const { rerender } = renderHook(
      ({ vector }: { vector: Float32Array }) =>
        useHybridSearchSubscribe('docs', 'query', { queryVector: vector }),
      {
        wrapper,
        initialProps: { vector: new Float32Array([0.1, 0.2]) },
      }
    );

    act(() => capturedCallback!([]));
    await waitFor(() => expect(mockHybridSearchSubscribe).toHaveBeenCalledTimes(1));

    // Change vector content — should trigger setOptions on existing handle
    rerender({ vector: new Float32Array([0.3, 0.4]) });
    // The existing handle receives setOptions; subscription count stays at 1
    expect(mockHybridSearchSubscribe).toHaveBeenCalledTimes(1);
    expect(mockSetOptions).toHaveBeenCalled();
  });

  // ============================================
  // AC #13 — unmounting disposes the handle
  // ============================================

  it('AC #13: unmounting the component disposes the handle', async () => {
    let capturedCallback: ((results: any[]) => void) | null = null;
    const handle = createMockHandle((cb) => {
      capturedCallback = cb;
      return () => {};
    });
    mockHybridSearchSubscribe.mockReturnValue(handle);

    const { result, unmount } = renderHook(
      () => useHybridSearchSubscribe('docs', 'query'),
      { wrapper }
    );

    act(() => capturedCallback!([]));
    await waitFor(() => expect(result.current.loading).toBe(false));

    unmount();

    expect(mockDispose).toHaveBeenCalled();
  });

  // ============================================
  // Additional: queryText change reuses existing handle via setQuery
  // ============================================

  it('reuses existing handle via setQuery when queryText changes', async () => {
    let capturedCallback: ((results: any[]) => void) | null = null;
    const handle = createMockHandle((cb) => {
      capturedCallback = cb;
      return () => {};
    });
    mockHybridSearchSubscribe.mockReturnValue(handle);

    const { rerender } = renderHook(
      ({ query }: { query: string }) => useHybridSearchSubscribe('docs', query),
      {
        wrapper,
        initialProps: { query: 'first' },
      }
    );

    act(() => capturedCallback!([]));
    await waitFor(() => expect(mockHybridSearchSubscribe).toHaveBeenCalledTimes(1));

    rerender({ query: 'second' });

    expect(mockSetQuery).toHaveBeenCalledWith('second');
    // Still only one subscription created
    expect(mockHybridSearchSubscribe).toHaveBeenCalledTimes(1);
  });
});
