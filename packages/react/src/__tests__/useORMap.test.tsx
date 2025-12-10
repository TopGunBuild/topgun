import { renderHook, act } from '@testing-library/react';
import { useORMap } from '../hooks/useORMap';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock ORMap (Observed-Remove Map)
const createMockORMap = () => {
  const data = new Map<string, Set<{ value: any; tag: string }>>();
  const tombstones = new Set<string>();
  const listeners = new Set<() => void>();

  let tagCounter = 0;

  return {
    add: jest.fn((key: string, value: any) => {
      if (!data.has(key)) {
        data.set(key, new Set());
      }
      const tag = `tag-${++tagCounter}`;
      data.get(key)!.add({ value, tag });
      listeners.forEach(cb => cb());
      return { value, tag, timestamp: Date.now() };
    }),
    get: jest.fn((key: string) => {
      const items = data.get(key);
      if (!items) return [];
      return Array.from(items).map(item => item.value);
    }),
    remove: jest.fn((key: string, value: any) => {
      const items = data.get(key);
      if (!items) return [];
      const removedTags: string[] = [];
      items.forEach(item => {
        if (item.value === value || JSON.stringify(item.value) === JSON.stringify(value)) {
          tombstones.add(item.tag);
          removedTags.push(item.tag);
          items.delete(item);
        }
      });
      listeners.forEach(cb => cb());
      return removedTags;
    }),
    has: jest.fn((key: string) => {
      const items = data.get(key);
      return items && items.size > 0;
    }),
    keys: jest.fn(() => Array.from(data.keys()).filter(k => data.get(k)!.size > 0)),
    onChange: jest.fn((callback: () => void) => {
      listeners.add(callback);
      return () => {
        listeners.delete(callback);
      };
    }),
    // Test helpers
    _triggerChange: () => {
      listeners.forEach(cb => cb());
    },
    _getListenerCount: () => listeners.size,
    _getTombstoneCount: () => tombstones.size,
  };
};

let mockORMap: ReturnType<typeof createMockORMap>;

const mockGetORMap = jest.fn(() => mockORMap);
const mockClient = {
  getORMap: mockGetORMap,
} as unknown as TopGunClient;

describe('useORMap', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockORMap = createMockORMap();
    mockGetORMap.mockReturnValue(mockORMap);
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should return an ORMap instance', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    expect(result.current).toBe(mockORMap);
    expect(mockGetORMap).toHaveBeenCalledWith('test-ormap');
  });

  it('should add value to a key', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    act(() => {
      result.current.add('tags', 'important');
    });

    expect(mockORMap.add).toHaveBeenCalledWith('tags', 'important');
  });

  it('should get all values for a key', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    act(() => {
      result.current.add('tags', 'important');
      result.current.add('tags', 'urgent');
    });

    expect(result.current.get('tags')).toEqual(['important', 'urgent']);
  });

  it('should remove value from a key (Observed-Remove semantics)', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    act(() => {
      result.current.add('tags', 'important');
      result.current.add('tags', 'urgent');
    });

    act(() => {
      result.current.remove('tags', 'important');
    });

    expect(mockORMap.remove).toHaveBeenCalledWith('tags', 'important');
    expect(result.current.get('tags')).toEqual(['urgent']);
  });

  it('should trigger re-render on map changes', () => {
    let renderCount = 0;
    const { result } = renderHook(() => {
      renderCount++;
      return useORMap('test-ormap');
    }, { wrapper });

    const initialRenderCount = renderCount;

    act(() => {
      mockORMap._triggerChange();
    });

    expect(renderCount).toBeGreaterThan(initialRenderCount);
  });

  it('should return empty array for non-existent keys', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    expect(result.current.get('non-existent')).toEqual([]);
  });

  it('should subscribe to onChange on mount', () => {
    renderHook(() => useORMap('test-ormap'), { wrapper });

    expect(mockORMap.onChange).toHaveBeenCalled();
    expect(mockORMap._getListenerCount()).toBe(1);
  });

  it('should unsubscribe from onChange on unmount', () => {
    const { unmount } = renderHook(() => useORMap('test-ormap'), { wrapper });

    expect(mockORMap._getListenerCount()).toBe(1);

    unmount();

    expect(mockORMap._getListenerCount()).toBe(0);
  });

  it('should allow adding multiple values to the same key', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    act(() => {
      result.current.add('colors', 'red');
      result.current.add('colors', 'blue');
      result.current.add('colors', 'green');
    });

    expect(mockORMap.add).toHaveBeenCalledTimes(3);
    expect(result.current.get('colors')).toEqual(['red', 'blue', 'green']);
  });

  it('should allow adding same value multiple times (multi-set semantics)', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    act(() => {
      result.current.add('items', 'apple');
      result.current.add('items', 'apple');
    });

    // ORMap allows duplicates with different tags
    expect(result.current.get('items')).toEqual(['apple', 'apple']);
  });

  it('should work with object values', () => {
    const { result } = renderHook(() => useORMap<string, { id: number; name: string }>('users'), { wrapper });

    const user1 = { id: 1, name: 'Alice' };
    const user2 = { id: 2, name: 'Bob' };

    act(() => {
      result.current.add('admins', user1);
      result.current.add('admins', user2);
    });

    expect(mockORMap.add).toHaveBeenCalledWith('admins', user1);
    expect(mockORMap.add).toHaveBeenCalledWith('admins', user2);
  });

  it('should handle remove operation with Observed-Remove semantics', () => {
    const { result } = renderHook(() => useORMap('test-ormap'), { wrapper });

    // Add some items
    act(() => {
      result.current.add('tags', 'a');
      result.current.add('tags', 'b');
      result.current.add('tags', 'c');
    });

    // Remove one
    act(() => {
      result.current.remove('tags', 'b');
    });

    // Only 'a' and 'c' should remain
    expect(result.current.get('tags')).toEqual(['a', 'c']);

    // Tombstone should be created
    expect(mockORMap._getTombstoneCount()).toBe(1);
  });

  it('should re-render when external changes occur', () => {
    let renderCount = 0;
    renderHook(() => {
      renderCount++;
      return useORMap('test-ormap');
    }, { wrapper });

    const countAfterMount = renderCount;

    // Simulate external change
    act(() => {
      mockORMap._triggerChange();
    });

    expect(renderCount).toBe(countAfterMount + 1);

    act(() => {
      mockORMap._triggerChange();
    });

    expect(renderCount).toBe(countAfterMount + 2);
  });
});
