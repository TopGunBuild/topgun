import { renderHook, act } from '@testing-library/react';
import { useWriteRejections } from '../hooks/useWriteRejections';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import type { WriteRejection } from '@topgunbuild/core';
import React from 'react';

// Mock client exposing only the refusal surface, plus a trigger helper.
const createMockClient = () => {
  let callback: ((rejection: WriteRejection) => void) | null = null;
  const client = {
    onWriteRejected: jest.fn((cb: (rejection: WriteRejection) => void) => {
      callback = cb;
      return () => {
        callback = null;
      };
    }),
    _trigger: (rejection: WriteRejection) => callback?.(rejection),
    _listening: () => callback !== null,
  };
  return client;
};

let mockClient: ReturnType<typeof createMockClient>;

const rejection = (id: string, overrides: Partial<WriteRejection> = {}): WriteRejection => ({
  id,
  mapName: 'todos',
  key: `key-${id}`,
  attemptedValue: { title: 'buy milk' },
  cause: 'forbidden',
  reason: 'write forbidden by policy',
  keptLocally: true,
  previouslyAcked: false,
  timestamp: { millis: 1000, counter: Number(id) || 0, nodeId: 'test-node' },
  ...overrides,
});

describe('useWriteRejections', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockClient = createMockClient();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient as unknown as TopGunClient}>{children}</TopGunProvider>
  );

  // The hook publishes on a microtask, so every trigger is flushed inside act().
  const emit = async (...events: WriteRejection[]) => {
    await act(async () => {
      for (const e of events) mockClient._trigger(e);
      await Promise.resolve();
    });
  };

  describe('initialization', () => {
    it('starts empty and subscribes on mount', () => {
      const { result } = renderHook(() => useWriteRejections(), { wrapper });

      expect(result.current.rejections).toEqual([]);
      expect(result.current.lastRejection).toBe(null);
      expect(mockClient.onWriteRejected).toHaveBeenCalled();
    });

    it('unsubscribes on unmount', () => {
      const { unmount } = renderHook(() => useWriteRejections(), { wrapper });
      expect(mockClient._listening()).toBe(true);

      unmount();
      expect(mockClient._listening()).toBe(false);
    });
  });

  describe('receiving refusals', () => {
    it('appends the refusal and exposes it as lastRejection', async () => {
      const { result } = renderHook(() => useWriteRejections(), { wrapper });

      await emit(rejection('1'));

      expect(result.current.rejections).toHaveLength(1);
      expect(result.current.rejections[0].id).toBe('1');
      expect(result.current.lastRejection?.id).toBe('1');
      expect(result.current.lastRejection?.cause).toBe('forbidden');
    });

    it('keeps refusals oldest-first', async () => {
      const { result } = renderHook(() => useWriteRejections(), { wrapper });

      await emit(rejection('1'));
      await emit(rejection('2'));

      expect(result.current.rejections.map((r) => r.id)).toEqual(['1', '2']);
      expect(result.current.lastRejection?.id).toBe('2');
    });

    it('filters by mapName when one is given', async () => {
      const { result } = renderHook(() => useWriteRejections({ mapName: 'todos' }), { wrapper });

      await emit(rejection('1'), rejection('2', { mapName: 'users' }));

      expect(result.current.rejections.map((r) => r.id)).toEqual(['1']);
    });
  });

  describe('bounds', () => {
    it('keeps at most maxHistory refusals, dropping the oldest', async () => {
      const { result } = renderHook(() => useWriteRejections({ maxHistory: 3 }), { wrapper });

      await emit(...['1', '2', '3', '4', '5'].map((id) => rejection(id)));

      expect(result.current.rejections.map((r) => r.id)).toEqual(['3', '4', '5']);
      expect(result.current.lastRejection?.id).toBe('5');
    });

    it('defaults maxHistory to 100', async () => {
      const { result } = renderHook(() => useWriteRejections(), { wrapper });

      await emit(...Array.from({ length: 150 }, (_, i) => rejection(String(i))));

      expect(result.current.rejections).toHaveLength(100);
      expect(result.current.rejections[0].id).toBe('50');
    });

    it('deduplicates by event id', async () => {
      const { result } = renderHook(() => useWriteRejections(), { wrapper });

      await emit(rejection('1'), rejection('1'));
      await emit(rejection('1'));

      expect(result.current.rejections.map((r) => r.id)).toEqual(['1']);
    });

    it('coalesces a reconnect burst into a single publish', async () => {
      // The mechanism, asserted directly: the burst schedules ONE drain, so the
      // hook copies its history once rather than 2,000 times. Render count is
      // not the predicate — React batches the synchronous burst either way.
      const scheduleSpy = jest.spyOn(global, 'queueMicrotask');
      let renders = 0;
      const { result } = renderHook(
        () => {
          renders += 1;
          return useWriteRejections({ maxHistory: 2000 });
        },
        { wrapper },
      );
      const rendersBefore = renders;
      let scheduledDuringBurst = -1;

      await act(async () => {
        scheduleSpy.mockClear();
        for (let i = 0; i < 2000; i += 1) mockClient._trigger(rejection(String(i)));
        // Read before awaiting: act's own flush schedules microtasks too.
        scheduledDuringBurst = scheduleSpy.mock.calls.length;
        await Promise.resolve();
      });

      expect(scheduledDuringBurst).toBe(1);
      expect(result.current.rejections).toHaveLength(2000);
      expect(renders - rendersBefore).toBe(1);
      scheduleSpy.mockRestore();
    });
  });

  describe('clear', () => {
    it('empties the history and lastRejection', async () => {
      const { result } = renderHook(() => useWriteRejections(), { wrapper });
      await emit(rejection('1'));

      act(() => {
        result.current.clear();
      });

      expect(result.current.rejections).toEqual([]);
      expect(result.current.lastRejection).toBe(null);
    });

    it('a refusal already cleared can be re-delivered and shown again', async () => {
      const { result } = renderHook(() => useWriteRejections(), { wrapper });
      await emit(rejection('1'));

      act(() => {
        result.current.clear();
      });
      await emit(rejection('1'));

      expect(result.current.rejections.map((r) => r.id)).toEqual(['1']);
    });
  });
});
