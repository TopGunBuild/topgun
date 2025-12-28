import { renderHook, act } from '@testing-library/react';
import { usePNCounter } from '../hooks/usePNCounter';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock PNCounterHandle
const createMockPNCounterHandle = () => {
  let currentValue = 0;
  const listeners: Array<(value: number) => void> = [];

  return {
    get: jest.fn(() => currentValue),
    increment: jest.fn(() => {
      currentValue++;
      listeners.forEach(l => l(currentValue));
      return currentValue;
    }),
    decrement: jest.fn(() => {
      currentValue--;
      listeners.forEach(l => l(currentValue));
      return currentValue;
    }),
    addAndGet: jest.fn((delta: number) => {
      currentValue += delta;
      listeners.forEach(l => l(currentValue));
      return currentValue;
    }),
    subscribe: jest.fn((cb) => {
      listeners.push(cb);
      cb(currentValue); // Immediately call with current value
      return () => {
        const idx = listeners.indexOf(cb);
        if (idx >= 0) listeners.splice(idx, 1);
      };
    }),
    // Test helpers
    setValue: (v: number) => {
      currentValue = v;
      listeners.forEach(l => l(currentValue));
    },
    getCurrentValue: () => currentValue,
  };
};

describe('usePNCounter', () => {
  let mockCounterHandle: ReturnType<typeof createMockPNCounterHandle>;
  let mockClient: TopGunClient;

  beforeEach(() => {
    jest.clearAllMocks();
    mockCounterHandle = createMockPNCounterHandle();
    mockClient = {
      getPNCounter: jest.fn().mockReturnValue(mockCounterHandle),
    } as unknown as TopGunClient;
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  describe('initialization', () => {
    it('should initialize with value 0 and loading false after subscribe', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      expect(result.current.value).toBe(0);
      expect(result.current.loading).toBe(false);
    });

    it('should call getPNCounter with the counter name', () => {
      renderHook(() => usePNCounter('likes:post-123'), { wrapper });

      expect(mockClient.getPNCounter).toHaveBeenCalledWith('likes:post-123');
    });

    it('should subscribe to counter updates', () => {
      renderHook(() => usePNCounter('testCounter'), { wrapper });

      expect(mockCounterHandle.subscribe).toHaveBeenCalled();
    });
  });

  describe('increment', () => {
    it('should call increment on the counter handle', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      act(() => {
        result.current.increment();
      });

      expect(mockCounterHandle.increment).toHaveBeenCalled();
    });

    it('should update value after increment', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      expect(result.current.value).toBe(0);

      act(() => {
        result.current.increment();
      });

      expect(result.current.value).toBe(1);

      act(() => {
        result.current.increment();
      });

      expect(result.current.value).toBe(2);
    });
  });

  describe('decrement', () => {
    it('should call decrement on the counter handle', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      act(() => {
        result.current.decrement();
      });

      expect(mockCounterHandle.decrement).toHaveBeenCalled();
    });

    it('should update value after decrement', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      // First increment a few times
      act(() => {
        result.current.increment();
        result.current.increment();
        result.current.increment();
      });

      expect(result.current.value).toBe(3);

      act(() => {
        result.current.decrement();
      });

      expect(result.current.value).toBe(2);
    });

    it('should allow negative values', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      act(() => {
        result.current.decrement();
      });

      expect(result.current.value).toBe(-1);
    });
  });

  describe('add', () => {
    it('should call addAndGet on the counter handle', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      act(() => {
        result.current.add(10);
      });

      expect(mockCounterHandle.addAndGet).toHaveBeenCalledWith(10);
    });

    it('should update value after add with positive delta', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      act(() => {
        result.current.add(10);
      });

      expect(result.current.value).toBe(10);
    });

    it('should update value after add with negative delta', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      act(() => {
        result.current.add(10);
      });

      act(() => {
        result.current.add(-3);
      });

      expect(result.current.value).toBe(7);
    });
  });

  describe('external updates', () => {
    it('should update value when counter emits new value', () => {
      const { result } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      expect(result.current.value).toBe(0);

      // Simulate external update (e.g., from server sync)
      act(() => {
        mockCounterHandle.setValue(42);
      });

      expect(result.current.value).toBe(42);
    });
  });

  describe('counter name changes', () => {
    it('should create new counter when name changes', () => {
      const { rerender } = renderHook(
        ({ name }) => usePNCounter(name),
        { wrapper, initialProps: { name: 'counter-1' } }
      );

      expect(mockClient.getPNCounter).toHaveBeenCalledWith('counter-1');

      rerender({ name: 'counter-2' });

      expect(mockClient.getPNCounter).toHaveBeenCalledWith('counter-2');
      expect(mockClient.getPNCounter).toHaveBeenCalledTimes(2);
    });
  });

  describe('cleanup', () => {
    it('should unsubscribe on unmount', () => {
      const { unmount } = renderHook(() => usePNCounter('testCounter'), { wrapper });

      // Get the unsubscribe function that was returned
      const unsubscribe = mockCounterHandle.subscribe.mock.results[0].value;
      const unsubscribeSpy = jest.fn(unsubscribe);

      // Replace the last subscribe call's return value with our spy
      mockCounterHandle.subscribe.mockReturnValueOnce(unsubscribeSpy);

      // Re-render to use our spy
      const { unmount: unmount2 } = renderHook(() => usePNCounter('anotherCounter'), { wrapper });

      unmount2();

      expect(unsubscribeSpy).toHaveBeenCalled();
    });
  });
});
