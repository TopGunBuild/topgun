import { renderHook, act } from '@testing-library/react';
import { useMergeRejections } from '../hooks/useMergeRejections';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import type { MergeRejection } from '@topgunbuild/core';
import React from 'react';

// Mock ConflictResolverClient with trigger helper
const createMockConflictResolvers = () => {
  let callback: ((rejection: MergeRejection) => void) | null = null;
  return {
    register: jest.fn().mockResolvedValue({ success: true }),
    unregister: jest.fn().mockResolvedValue({ success: true }),
    list: jest.fn().mockResolvedValue([]),
    onRejection: jest.fn((cb: (rejection: MergeRejection) => void) => {
      callback = cb;
      return () => {
        callback = null;
      };
    }),
    _triggerRejection: (rejection: MergeRejection) => callback?.(rejection),
  };
};

let mockConflictResolvers: ReturnType<typeof createMockConflictResolvers>;

const mockClient = {
  getConflictResolvers: jest.fn(() => mockConflictResolvers),
} as unknown as TopGunClient;

// Helper to create test rejection
const createRejection = (
  mapName: string,
  key: string,
  reason: string,
  millis: number = Date.now(),
  counter: number = 0
): MergeRejection => ({
  mapName,
  key,
  attemptedValue: { value: 'test' },
  reason,
  timestamp: { millis, counter, nodeId: 'test-node' },
  nodeId: 'test-node',
});

describe('useMergeRejections', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockConflictResolvers = createMockConflictResolvers();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  describe('Initialization', () => {
    it('should initialize with rejections=[], lastRejection=null', () => {
      const { result } = renderHook(() => useMergeRejections(), { wrapper });

      expect(result.current.rejections).toEqual([]);
      expect(result.current.lastRejection).toBe(null);
    });

    it('should subscribe to onRejection on mount', () => {
      renderHook(() => useMergeRejections(), { wrapper });

      expect(mockConflictResolvers.onRejection).toHaveBeenCalled();
    });
  });

  describe('Receiving rejections', () => {
    it('should add rejection to rejections array', () => {
      const { result } = renderHook(() => useMergeRejections(), { wrapper });

      const rejection = createRejection('testMap', 'key1', 'Conflict detected');

      act(() => {
        mockConflictResolvers._triggerRejection(rejection);
      });

      expect(result.current.rejections).toHaveLength(1);
      expect(result.current.rejections[0]).toEqual(rejection);
    });

    it('should update lastRejection when rejection received', () => {
      const { result } = renderHook(() => useMergeRejections(), { wrapper });

      const rejection1 = createRejection('testMap', 'key1', 'First conflict');
      const rejection2 = createRejection('testMap', 'key2', 'Second conflict');

      act(() => {
        mockConflictResolvers._triggerRejection(rejection1);
      });

      expect(result.current.lastRejection).toEqual(rejection1);

      act(() => {
        mockConflictResolvers._triggerRejection(rejection2);
      });

      expect(result.current.lastRejection).toEqual(rejection2);
    });
  });

  describe('Filtering by mapName', () => {
    it('should only include rejections matching mapName when specified', () => {
      const { result } = renderHook(
        () => useMergeRejections({ mapName: 'targetMap' }),
        { wrapper }
      );

      const rejection1 = createRejection('targetMap', 'key1', 'Should be included');
      const rejection2 = createRejection('otherMap', 'key2', 'Should be filtered out');
      const rejection3 = createRejection('targetMap', 'key3', 'Should be included');

      act(() => {
        mockConflictResolvers._triggerRejection(rejection1);
        mockConflictResolvers._triggerRejection(rejection2);
        mockConflictResolvers._triggerRejection(rejection3);
      });

      expect(result.current.rejections).toHaveLength(2);
      expect(result.current.rejections[0]).toEqual(rejection1);
      expect(result.current.rejections[1]).toEqual(rejection3);
    });

    it('should include all rejections when mapName not specified', () => {
      const { result } = renderHook(() => useMergeRejections(), { wrapper });

      const rejection1 = createRejection('map1', 'key1', 'Conflict 1');
      const rejection2 = createRejection('map2', 'key2', 'Conflict 2');

      act(() => {
        mockConflictResolvers._triggerRejection(rejection1);
        mockConflictResolvers._triggerRejection(rejection2);
      });

      expect(result.current.rejections).toHaveLength(2);
      expect(result.current.rejections[0]).toEqual(rejection1);
      expect(result.current.rejections[1]).toEqual(rejection2);
    });
  });

  describe('maxHistory', () => {
    it('should limit rejections array to maxHistory (default 100)', () => {
      const { result } = renderHook(() => useMergeRejections(), { wrapper });

      // Add 150 rejections
      act(() => {
        for (let i = 0; i < 150; i++) {
          mockConflictResolvers._triggerRejection(
            createRejection('testMap', `key${i}`, `Rejection ${i}`, Date.now() + i, i)
          );
        }
      });

      // Should only keep the last 100
      expect(result.current.rejections).toHaveLength(100);
      // First item should be from index 50 (150 - 100)
      expect(result.current.rejections[0].key).toBe('key50');
      // Last item should be from index 149
      expect(result.current.rejections[99].key).toBe('key149');
    });

    it('should keep most recent rejections when limit exceeded', () => {
      const { result } = renderHook(
        () => useMergeRejections({ maxHistory: 5 }),
        { wrapper }
      );

      // Add 10 rejections
      act(() => {
        for (let i = 0; i < 10; i++) {
          mockConflictResolvers._triggerRejection(
            createRejection('testMap', `key${i}`, `Rejection ${i}`, Date.now() + i, i)
          );
        }
      });

      // Should only keep the last 5
      expect(result.current.rejections).toHaveLength(5);
      expect(result.current.rejections[0].key).toBe('key5');
      expect(result.current.rejections[4].key).toBe('key9');
    });
  });

  describe('clear()', () => {
    it('should clear rejections array and lastRejection', () => {
      const { result } = renderHook(() => useMergeRejections(), { wrapper });

      // Add some rejections
      act(() => {
        mockConflictResolvers._triggerRejection(createRejection('testMap', 'key1', 'Conflict 1'));
        mockConflictResolvers._triggerRejection(createRejection('testMap', 'key2', 'Conflict 2'));
      });

      expect(result.current.rejections).toHaveLength(2);
      expect(result.current.lastRejection).not.toBe(null);

      // Clear the rejections
      act(() => {
        result.current.clear();
      });

      expect(result.current.rejections).toEqual([]);
      expect(result.current.lastRejection).toBe(null);
    });
  });

  describe('Cleanup', () => {
    it('should unsubscribe on unmount', () => {
      const { unmount } = renderHook(() => useMergeRejections(), { wrapper });

      const unsubscribeSpy = jest.fn();
      mockConflictResolvers.onRejection.mockReturnValue(unsubscribeSpy);

      // Re-render to get the spy in place
      const { unmount: unmount2 } = renderHook(() => useMergeRejections(), { wrapper });

      unmount2();

      expect(unsubscribeSpy).toHaveBeenCalled();
    });

    it('should not receive rejections after unmount', () => {
      const { result, unmount } = renderHook(() => useMergeRejections(), { wrapper });

      // Add a rejection before unmount
      act(() => {
        mockConflictResolvers._triggerRejection(createRejection('testMap', 'key1', 'Before unmount'));
      });

      expect(result.current.rejections).toHaveLength(1);

      unmount();

      // Try to trigger after unmount - should not affect the hook
      act(() => {
        mockConflictResolvers._triggerRejection(createRejection('testMap', 'key2', 'After unmount'));
      });

      // Still only 1 rejection (the one before unmount)
      expect(result.current.rejections).toHaveLength(1);
    });
  });
});
