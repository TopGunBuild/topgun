import { renderHook, act } from '@testing-library/react';
import { useMap } from '../hooks/useMap';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock LWWMap
const createMockLWWMap = () => {
  const data = new Map<string, any>();
  const listeners = new Set<() => void>();

  return {
    get: jest.fn((key: string) => data.get(key)),
    set: jest.fn((key: string, value: any) => {
      data.set(key, value);
      listeners.forEach(cb => cb());
      return { value, timestamp: Date.now(), deleted: false };
    }),
    remove: jest.fn((key: string) => {
      data.delete(key);
      listeners.forEach(cb => cb());
      return { value: null, timestamp: Date.now(), deleted: true };
    }),
    has: jest.fn((key: string) => data.has(key)),
    keys: jest.fn(() => Array.from(data.keys())),
    values: jest.fn(() => Array.from(data.values())),
    entries: jest.fn(() => Array.from(data.entries())),
    onChange: jest.fn((callback: () => void) => {
      listeners.add(callback);
      return () => {
        listeners.delete(callback);
      };
    }),
    // Helper to trigger changes externally
    _triggerChange: () => {
      listeners.forEach(cb => cb());
    },
    _set: (key: string, value: any) => {
      data.set(key, value);
    },
    _getListenerCount: () => listeners.size,
  };
};

let mockMap: ReturnType<typeof createMockLWWMap>;

const mockGetMap = jest.fn(() => mockMap);
const mockClient = {
  getMap: mockGetMap,
} as unknown as TopGunClient;

describe('useMap', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockMap = createMockLWWMap();
    mockGetMap.mockReturnValue(mockMap);
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should return a map instance', () => {
    const { result } = renderHook(() => useMap('test-map'), { wrapper });

    expect(result.current).toBe(mockMap);
    expect(mockGetMap).toHaveBeenCalledWith('test-map');
  });

  it('should get value by key', () => {
    mockMap._set('key1', 'value1');

    const { result } = renderHook(() => useMap('test-map'), { wrapper });

    expect(result.current.get('key1')).toBe('value1');
    expect(mockMap.get).toHaveBeenCalledWith('key1');
  });

  it('should set value', () => {
    const { result } = renderHook(() => useMap('test-map'), { wrapper });

    act(() => {
      result.current.set('key1', 'value1');
    });

    expect(mockMap.set).toHaveBeenCalledWith('key1', 'value1');
  });

  it('should remove value', () => {
    const { result } = renderHook(() => useMap('test-map'), { wrapper });

    act(() => {
      result.current.remove('key1');
    });

    expect(mockMap.remove).toHaveBeenCalledWith('key1');
  });

  it('should trigger re-render on map changes', () => {
    let renderCount = 0;
    const { result } = renderHook(() => {
      renderCount++;
      return useMap('test-map');
    }, { wrapper });

    const initialRenderCount = renderCount;

    act(() => {
      mockMap._triggerChange();
    });

    expect(renderCount).toBeGreaterThan(initialRenderCount);
  });

  it('should return undefined for non-existent keys', () => {
    const { result } = renderHook(() => useMap('test-map'), { wrapper });

    expect(result.current.get('non-existent')).toBeUndefined();
  });

  it('should subscribe to onChange on mount', () => {
    renderHook(() => useMap('test-map'), { wrapper });

    expect(mockMap.onChange).toHaveBeenCalled();
    expect(mockMap._getListenerCount()).toBe(1);
  });

  it('should unsubscribe from onChange on unmount', () => {
    const { unmount } = renderHook(() => useMap('test-map'), { wrapper });

    expect(mockMap._getListenerCount()).toBe(1);

    unmount();

    expect(mockMap._getListenerCount()).toBe(0);
  });

  it('should handle multiple set operations', () => {
    const { result } = renderHook(() => useMap('test-map'), { wrapper });

    act(() => {
      result.current.set('key1', 'value1');
      result.current.set('key2', 'value2');
      result.current.set('key3', 'value3');
    });

    expect(mockMap.set).toHaveBeenCalledTimes(3);
    expect(mockMap.set).toHaveBeenCalledWith('key1', 'value1');
    expect(mockMap.set).toHaveBeenCalledWith('key2', 'value2');
    expect(mockMap.set).toHaveBeenCalledWith('key3', 'value3');
  });

  it('should work with typed keys and values', () => {
    const typedMockMap = createMockLWWMap();
    mockGetMap.mockReturnValue(typedMockMap);

    const { result } = renderHook(() => useMap<string, { name: string; age: number }>('users'), { wrapper });

    act(() => {
      result.current.set('user1', { name: 'John', age: 30 });
    });

    expect(typedMockMap.set).toHaveBeenCalledWith('user1', { name: 'John', age: 30 });
  });

  it('should update value for existing key', () => {
    mockMap._set('key1', 'oldValue');

    const { result } = renderHook(() => useMap('test-map'), { wrapper });

    act(() => {
      result.current.set('key1', 'newValue');
    });

    expect(mockMap.set).toHaveBeenCalledWith('key1', 'newValue');
  });
});
