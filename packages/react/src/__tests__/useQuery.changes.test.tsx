import { renderHook, act, waitFor } from '@testing-library/react';
import { useQuery } from '../hooks/useQuery';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient, ChangeEvent } from '@topgunbuild/client';
import React from 'react';

// Mock QueryHandle with change tracking support
const createMockQueryHandle = () => {
  const dataListeners: Array<(results: any[]) => void> = [];
  const changeListeners: Array<(changes: ChangeEvent<any>[]) => void> = [];

  return {
    subscribe: jest.fn((cb) => {
      dataListeners.push(cb);
      return () => {
        const idx = dataListeners.indexOf(cb);
        if (idx >= 0) dataListeners.splice(idx, 1);
      };
    }),
    onChanges: jest.fn((cb) => {
      changeListeners.push(cb);
      return () => {
        const idx = changeListeners.indexOf(cb);
        if (idx >= 0) changeListeners.splice(idx, 1);
      };
    }),
    // Test helpers
    emitData: (data: any[]) => {
      for (const cb of dataListeners) cb(data);
    },
    emitChanges: (changes: ChangeEvent<any>[]) => {
      for (const cb of changeListeners) cb(changes);
    },
  };
};

describe('useQuery with change tracking', () => {
  let mockQueryHandle: ReturnType<typeof createMockQueryHandle>;
  let mockClient: TopGunClient;

  beforeEach(() => {
    jest.clearAllMocks();
    mockQueryHandle = createMockQueryHandle();
    mockClient = {
      query: jest.fn().mockReturnValue(mockQueryHandle),
    } as unknown as TopGunClient;
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  describe('change state', () => {
    it('should initialize with empty changes and null lastChange', () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      expect(result.current.changes).toEqual([]);
      expect(result.current.lastChange).toBeNull();
    });

    it('should provide lastChange on data update', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      const addChange: ChangeEvent<any> = {
        type: 'add',
        key: 'item-1',
        value: { title: 'Test Todo' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitData([{ _key: 'item-1', title: 'Test Todo' }]);
        mockQueryHandle.emitChanges([addChange]);
      });

      expect(result.current.lastChange?.type).toBe('add');
      expect(result.current.lastChange?.key).toBe('item-1');
      expect(result.current.lastChange?.value).toEqual({ title: 'Test Todo' });
    });

    it('should accumulate changes array', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      const addChange1: ChangeEvent<any> = {
        type: 'add',
        key: 'item-1',
        value: { title: 'Todo 1' },
        timestamp: 1,
      };

      const addChange2: ChangeEvent<any> = {
        type: 'add',
        key: 'item-2',
        value: { title: 'Todo 2' },
        timestamp: 2,
      };

      act(() => {
        mockQueryHandle.emitData([{ _key: 'item-1', title: 'Todo 1' }]);
        mockQueryHandle.emitChanges([addChange1]);
      });

      act(() => {
        mockQueryHandle.emitData([
          { _key: 'item-1', title: 'Todo 1' },
          { _key: 'item-2', title: 'Todo 2' },
        ]);
        mockQueryHandle.emitChanges([addChange2]);
      });

      expect(result.current.changes).toHaveLength(2);
      expect(result.current.changes[0].key).toBe('item-1');
      expect(result.current.changes[1].key).toBe('item-2');
      expect(result.current.lastChange?.key).toBe('item-2');
    });

    it('should handle update change type', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      const updateChange: ChangeEvent<any> = {
        type: 'update',
        key: 'item-1',
        value: { title: 'Updated Title' },
        previousValue: { title: 'Original Title' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitData([{ _key: 'item-1', title: 'Updated Title' }]);
        mockQueryHandle.emitChanges([updateChange]);
      });

      expect(result.current.lastChange?.type).toBe('update');
      expect(result.current.lastChange?.value).toEqual({ title: 'Updated Title' });
      expect(result.current.lastChange?.previousValue).toEqual({ title: 'Original Title' });
    });

    it('should handle remove change type', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      const removeChange: ChangeEvent<any> = {
        type: 'remove',
        key: 'item-1',
        previousValue: { title: 'Deleted Todo' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitData([]);
        mockQueryHandle.emitChanges([removeChange]);
      });

      expect(result.current.lastChange?.type).toBe('remove');
      expect(result.current.lastChange?.key).toBe('item-1');
      expect(result.current.lastChange?.previousValue).toEqual({ title: 'Deleted Todo' });
    });
  });

  describe('clearChanges', () => {
    it('should clear changes when clearChanges is called', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      const addChange: ChangeEvent<any> = {
        type: 'add',
        key: 'item-1',
        value: { title: 'Test' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitData([{ _key: 'item-1', title: 'Test' }]);
        mockQueryHandle.emitChanges([addChange]);
      });

      expect(result.current.changes).toHaveLength(1);
      expect(result.current.lastChange).not.toBeNull();

      act(() => {
        result.current.clearChanges();
      });

      expect(result.current.changes).toHaveLength(0);
      expect(result.current.lastChange).toBeNull();
    });

    it('should allow new changes to accumulate after clearing', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      const change1: ChangeEvent<any> = {
        type: 'add',
        key: 'item-1',
        value: { title: 'First' },
        timestamp: 1,
      };

      act(() => {
        mockQueryHandle.emitChanges([change1]);
      });

      act(() => {
        result.current.clearChanges();
      });

      const change2: ChangeEvent<any> = {
        type: 'add',
        key: 'item-2',
        value: { title: 'Second' },
        timestamp: 2,
      };

      act(() => {
        mockQueryHandle.emitChanges([change2]);
      });

      expect(result.current.changes).toHaveLength(1);
      expect(result.current.changes[0].key).toBe('item-2');
    });
  });

  describe('callback options', () => {
    it('should call onChange callback for all changes', async () => {
      const onChange = jest.fn();

      const { result } = renderHook(
        () => useQuery('testMap', {}, { onChange }),
        { wrapper }
      );

      const changes: ChangeEvent<any>[] = [
        { type: 'add', key: 'item-1', value: { title: 'Added' }, timestamp: 1 },
        { type: 'update', key: 'item-2', value: { title: 'Updated' }, previousValue: { title: 'Old' }, timestamp: 2 },
      ];

      act(() => {
        mockQueryHandle.emitChanges(changes);
      });

      expect(onChange).toHaveBeenCalledTimes(2);
      expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ type: 'add', key: 'item-1' }));
      expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ type: 'update', key: 'item-2' }));
    });

    it('should call onAdd callback for add events', async () => {
      const onAdd = jest.fn();

      renderHook(
        () => useQuery('testMap', {}, { onAdd }),
        { wrapper }
      );

      const addChange: ChangeEvent<any> = {
        type: 'add',
        key: 'item-1',
        value: { title: 'New Todo' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitChanges([addChange]);
      });

      expect(onAdd).toHaveBeenCalledTimes(1);
      expect(onAdd).toHaveBeenCalledWith('item-1', { title: 'New Todo' });
    });

    it('should call onUpdate callback for update events', async () => {
      const onUpdate = jest.fn();

      renderHook(
        () => useQuery('testMap', {}, { onUpdate }),
        { wrapper }
      );

      const updateChange: ChangeEvent<any> = {
        type: 'update',
        key: 'item-1',
        value: { title: 'Updated' },
        previousValue: { title: 'Original' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitChanges([updateChange]);
      });

      expect(onUpdate).toHaveBeenCalledTimes(1);
      expect(onUpdate).toHaveBeenCalledWith('item-1', { title: 'Updated' }, { title: 'Original' });
    });

    it('should call onRemove callback for remove events', async () => {
      const onRemove = jest.fn();

      renderHook(
        () => useQuery('testMap', {}, { onRemove }),
        { wrapper }
      );

      const removeChange: ChangeEvent<any> = {
        type: 'remove',
        key: 'item-1',
        previousValue: { title: 'Deleted' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitChanges([removeChange]);
      });

      expect(onRemove).toHaveBeenCalledTimes(1);
      expect(onRemove).toHaveBeenCalledWith('item-1', { title: 'Deleted' });
    });

    it('should not call specific callbacks for other event types', async () => {
      const onAdd = jest.fn();
      const onUpdate = jest.fn();
      const onRemove = jest.fn();

      renderHook(
        () => useQuery('testMap', {}, { onAdd, onUpdate, onRemove }),
        { wrapper }
      );

      const addChange: ChangeEvent<any> = {
        type: 'add',
        key: 'item-1',
        value: { title: 'Added' },
        timestamp: 1,
      };

      act(() => {
        mockQueryHandle.emitChanges([addChange]);
      });

      expect(onAdd).toHaveBeenCalledTimes(1);
      expect(onUpdate).not.toHaveBeenCalled();
      expect(onRemove).not.toHaveBeenCalled();
    });
  });

  describe('query changes reset', () => {
    it('should reset changes when query changes', async () => {
      const { result, rerender } = renderHook(
        ({ mapName }) => useQuery(mapName),
        { wrapper, initialProps: { mapName: 'todos' } }
      );

      const addChange: ChangeEvent<any> = {
        type: 'add',
        key: 'item-1',
        value: { title: 'Test' },
        timestamp: Date.now(),
      };

      act(() => {
        mockQueryHandle.emitChanges([addChange]);
      });

      expect(result.current.changes).toHaveLength(1);

      // Change the query
      rerender({ mapName: 'notes' });

      // Changes should be reset
      expect(result.current.changes).toHaveLength(0);
      expect(result.current.lastChange).toBeNull();
    });
  });

  describe('batch changes', () => {
    it('should handle batch changes in single emission', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      const batchChanges: ChangeEvent<any>[] = [
        { type: 'add', key: 'item-1', value: { title: 'First' }, timestamp: 1 },
        { type: 'add', key: 'item-2', value: { title: 'Second' }, timestamp: 2 },
        { type: 'add', key: 'item-3', value: { title: 'Third' }, timestamp: 3 },
      ];

      act(() => {
        mockQueryHandle.emitChanges(batchChanges);
      });

      expect(result.current.changes).toHaveLength(3);
      expect(result.current.lastChange?.key).toBe('item-3');
    });
  });

  describe('maxChanges option', () => {
    it('should rotate oldest changes when exceeding maxChanges limit', async () => {
      const { result } = renderHook(
        () => useQuery('testMap', {}, { maxChanges: 3 }),
        { wrapper }
      );

      // Emit 5 changes, exceeding the limit of 3
      for (let i = 1; i <= 5; i++) {
        act(() => {
          mockQueryHandle.emitChanges([
            { type: 'add', key: `item-${i}`, value: { title: `Item ${i}` }, timestamp: i },
          ]);
        });
      }

      // Should only keep the last 3 changes
      expect(result.current.changes).toHaveLength(3);
      expect(result.current.changes[0].key).toBe('item-3');
      expect(result.current.changes[1].key).toBe('item-4');
      expect(result.current.changes[2].key).toBe('item-5');
    });

    it('should use default maxChanges of 1000', async () => {
      const { result } = renderHook(() => useQuery('testMap'), { wrapper });

      // Emit a batch that doesn't exceed default limit
      const changes: ChangeEvent<any>[] = [];
      for (let i = 0; i < 100; i++) {
        changes.push({ type: 'add', key: `item-${i}`, value: { title: `Item ${i}` }, timestamp: i });
      }

      act(() => {
        mockQueryHandle.emitChanges(changes);
      });

      // All 100 should be kept (under 1000 limit)
      expect(result.current.changes).toHaveLength(100);
    });

    it('should handle batch emission exceeding maxChanges', async () => {
      const { result } = renderHook(
        () => useQuery('testMap', {}, { maxChanges: 5 }),
        { wrapper }
      );

      // Emit 10 changes at once
      const batchChanges: ChangeEvent<any>[] = [];
      for (let i = 1; i <= 10; i++) {
        batchChanges.push({ type: 'add', key: `item-${i}`, value: { title: `Item ${i}` }, timestamp: i });
      }

      act(() => {
        mockQueryHandle.emitChanges(batchChanges);
      });

      // Should only keep the last 5
      expect(result.current.changes).toHaveLength(5);
      expect(result.current.changes[0].key).toBe('item-6');
      expect(result.current.changes[4].key).toBe('item-10');
    });
  });
});
