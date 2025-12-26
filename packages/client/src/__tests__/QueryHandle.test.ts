import { QueryHandle } from '../QueryHandle';
import { SyncEngine } from '../SyncEngine';

// Mock SyncEngine
const mockSyncEngine = {
  subscribeToQuery: jest.fn(),
  unsubscribeFromQuery: jest.fn(),
  runLocalQuery: jest.fn().mockResolvedValue([]),
} as unknown as SyncEngine;

describe('QueryHandle', () => {
  test('should sort results on client side', () => {
    const handle = new QueryHandle<any>(mockSyncEngine, 'items', {
      sort: { score: 'desc' }
    });

    const callback = jest.fn();
    handle.subscribe(callback);

    // Simulate receiving unsorted data from server
    handle.onResult([
      { key: 'A', value: { id: 'A', score: 10 } },
      { key: 'B', value: { id: 'B', score: 30 } },
      { key: 'C', value: { id: 'C', score: 20 } },
    ], 'server');

    expect(callback).toHaveBeenCalled();
    const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];

    expect(lastCall).toHaveLength(3);
    expect(lastCall[0].score).toBe(30); // B
    expect(lastCall[0]._key).toBe('B');
    expect(lastCall[1].score).toBe(20); // C
    expect(lastCall[1]._key).toBe('C');
    expect(lastCall[2].score).toBe(10); // A
    expect(lastCall[2]._key).toBe('A');
  });

  test('should maintain sort order on updates', () => {
    const handle = new QueryHandle<any>(mockSyncEngine, 'items', {
      sort: { score: 'desc' }
    });

    const callback = jest.fn();
    handle.subscribe(callback);

    // Initial server response
    handle.onResult([
      { key: 'A', value: { id: 'A', score: 10 } },
      { key: 'B', value: { id: 'B', score: 30 } },
    ], 'server');

    // Update A to be 40 (should move to top)
    handle.onUpdate('A', { id: 'A', score: 40 });

    const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
    expect(lastCall[0].score).toBe(40); // A
    expect(lastCall[0]._key).toBe('A');
    expect(lastCall[1].score).toBe(30); // B
    expect(lastCall[1]._key).toBe('B');
  });

  test('should include _key in results', () => {
    const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

    const callback = jest.fn();
    handle.subscribe(callback);

    handle.onResult([
      { key: 'node-123', value: { name: 'Node 1' } },
      { key: 'node-456', value: { name: 'Node 2' } },
    ], 'server');

    const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
    expect(lastCall[0]._key).toBe('node-123');
    expect(lastCall[0].name).toBe('Node 1');
    expect(lastCall[1]._key).toBe('node-456');
    expect(lastCall[1].name).toBe('Node 2');
  });

  describe('Race condition protection', () => {
    test('should ignore empty server response before receiving authoritative data', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // First: local data loads
      handle.onResult([
        { key: 'A', value: { name: 'Local Item A' } },
        { key: 'B', value: { name: 'Local Item B' } },
      ], 'local');

      // Server sends empty response (race condition - server hasn't loaded from storage yet)
      handle.onResult([], 'server');

      // Local data should be preserved
      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(2);
      expect(lastCall[0]._key).toBe('A');
      expect(lastCall[1]._key).toBe('B');
    });

    test('should accept empty server response after receiving non-empty server data', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // First: server sends data (authoritative)
      handle.onResult([
        { key: 'A', value: { name: 'Server Item A' } },
      ], 'server');

      // Later: server sends empty (all data was deleted)
      handle.onResult([], 'server');

      // Data should be cleared
      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(0);
    });

    test('should replace local data when server sends non-empty response', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // Local data first
      handle.onResult([
        { key: 'local-only', value: { name: 'Local Only' } },
      ], 'local');

      // Server sends different data
      handle.onResult([
        { key: 'server-item', value: { name: 'Server Item' } },
      ], 'server');

      // Server data should replace local
      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(1);
      expect(lastCall[0]._key).toBe('server-item');
    });

    test('should handle In-Memory adapter scenario correctly', () => {
      // Simulates: In-Memory server has no data, but client has local IndexedDB data
      const handle = new QueryHandle<any>(mockSyncEngine, 'notes:user123', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // Client loads from IndexedDB
      handle.onResult([
        { key: 'note1', value: { title: 'My Note', content: 'Content' } },
      ], 'local');

      // In-Memory server responds empty (it has no persistent data)
      handle.onResult([], 'server');

      // Local data should be preserved (not cleared!)
      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(1);
      expect(lastCall[0].title).toBe('My Note');
    });
  });

  describe('Change tracking (Phase 5.1)', () => {
    let handle: QueryHandle<{ name: string }>;

    beforeEach(() => {
      handle = new QueryHandle<{ name: string }>(mockSyncEngine, 'items', {});
      handle.subscribe(() => {}); // Activate handle
    });

    describe('onChanges', () => {
      test('should subscribe to change events', () => {
        const changeListener = jest.fn();
        const unsubscribe = handle.onChanges(changeListener);

        handle.onResult([
          { key: 'a', value: { name: 'Alice' } },
        ], 'server');

        expect(changeListener).toHaveBeenCalledTimes(1);
        expect(changeListener).toHaveBeenCalledWith(
          expect.arrayContaining([
            expect.objectContaining({ type: 'add', key: 'a', value: { name: 'Alice' } })
          ])
        );

        unsubscribe();
      });

      test('should unsubscribe from change events', () => {
        const changeListener = jest.fn();
        const unsubscribe = handle.onChanges(changeListener);

        // First update - should trigger listener
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        expect(changeListener).toHaveBeenCalledTimes(1);

        // Unsubscribe
        unsubscribe();

        // Second update - should not trigger listener
        handle.onUpdate('a', { name: 'Alice Updated' });
        expect(changeListener).toHaveBeenCalledTimes(1);
      });

      test('should support multiple change listeners', () => {
        const listener1 = jest.fn();
        const listener2 = jest.fn();

        handle.onChanges(listener1);
        handle.onChanges(listener2);

        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        expect(listener1).toHaveBeenCalledTimes(1);
        expect(listener2).toHaveBeenCalledTimes(1);
      });

      test('should catch errors in change listeners without affecting others', () => {
        const errorListener = jest.fn(() => { throw new Error('Test error'); });
        const normalListener = jest.fn();

        handle.onChanges(errorListener);
        handle.onChanges(normalListener);

        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        expect(errorListener).toHaveBeenCalled();
        expect(normalListener).toHaveBeenCalled();
      });
    });

    describe('consumeChanges', () => {
      test('should return and clear pending changes', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.onUpdate('b', { name: 'Bob' });

        const changes = handle.consumeChanges();

        expect(changes).toHaveLength(2);
        expect(changes[0]).toMatchObject({ type: 'add', key: 'a' });
        expect(changes[1]).toMatchObject({ type: 'add', key: 'b' });

        // Subsequent consume should return empty
        const nextChanges = handle.consumeChanges();
        expect(nextChanges).toHaveLength(0);
      });

      test('should return empty array when no changes', () => {
        const changes = handle.consumeChanges();
        expect(changes).toEqual([]);
      });

      test('should not affect change listeners', () => {
        const listener = jest.fn();
        handle.onChanges(listener);

        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.consumeChanges();

        // Listener should still have been called
        expect(listener).toHaveBeenCalledTimes(1);
      });
    });

    describe('getLastChange', () => {
      test('should return last change without consuming', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.onUpdate('a', { name: 'Alice Updated' });

        const lastChange = handle.getLastChange();

        expect(lastChange).toMatchObject({
          type: 'update',
          key: 'a',
          value: { name: 'Alice Updated' },
          previousValue: { name: 'Alice' }
        });

        // Calling again should return the same
        expect(handle.getLastChange()).toEqual(lastChange);
      });

      test('should return null when no pending changes', () => {
        expect(handle.getLastChange()).toBeNull();
      });

      test('should return null after consuming all changes', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.consumeChanges();

        expect(handle.getLastChange()).toBeNull();
      });
    });

    describe('getPendingChanges', () => {
      test('should return copy of pending changes without consuming', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.onUpdate('b', { name: 'Bob' });

        const pending1 = handle.getPendingChanges();
        const pending2 = handle.getPendingChanges();

        expect(pending1).toHaveLength(2);
        expect(pending2).toHaveLength(2);
        expect(pending1).not.toBe(pending2); // Different array instances
        expect(pending1).toEqual(pending2);  // Same content
      });

      test('should return empty array when no changes', () => {
        expect(handle.getPendingChanges()).toEqual([]);
      });
    });

    describe('clearChanges', () => {
      test('should clear all pending changes', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        expect(handle.getPendingChanges()).toHaveLength(1);

        handle.clearChanges();

        expect(handle.getPendingChanges()).toHaveLength(0);
        expect(handle.getLastChange()).toBeNull();
      });
    });

    describe('resetChangeTracker', () => {
      test('should reset tracker and clear pending changes', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        handle.resetChangeTracker();

        expect(handle.getPendingChanges()).toHaveLength(0);

        // After reset, same data should be detected as additions again
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        const changes = handle.getPendingChanges();
        expect(changes).toHaveLength(1);
        expect(changes[0].type).toBe('add');
      });
    });

    describe('change detection scenarios', () => {
      test('should detect add changes', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        const changes = handle.getPendingChanges();
        expect(changes).toHaveLength(1);
        expect(changes[0]).toMatchObject({
          type: 'add',
          key: 'a',
          value: { name: 'Alice' }
        });
        expect(changes[0].previousValue).toBeUndefined();
      });

      test('should detect update changes', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.consumeChanges();

        handle.onUpdate('a', { name: 'Alice Updated' });

        const changes = handle.getPendingChanges();
        expect(changes).toHaveLength(1);
        expect(changes[0]).toMatchObject({
          type: 'update',
          key: 'a',
          value: { name: 'Alice Updated' },
          previousValue: { name: 'Alice' }
        });
      });

      test('should detect remove changes', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.consumeChanges();

        handle.onUpdate('a', null); // null = deletion

        const changes = handle.getPendingChanges();
        expect(changes).toHaveLength(1);
        expect(changes[0]).toMatchObject({
          type: 'remove',
          key: 'a',
          previousValue: { name: 'Alice' }
        });
        expect(changes[0].value).toBeUndefined();
      });

      test('should detect mixed changes in batch', () => {
        handle.onResult([
          { key: 'a', value: { name: 'Alice' } },
          { key: 'b', value: { name: 'Bob' } },
        ], 'server');
        handle.consumeChanges();

        // a: updated, b: removed, c: added
        handle.onResult([
          { key: 'a', value: { name: 'Alice Updated' } },
          { key: 'c', value: { name: 'Charlie' } },
        ], 'server');

        const changes = handle.getPendingChanges();
        expect(changes).toHaveLength(3);

        const update = changes.find(c => c.type === 'update');
        const add = changes.find(c => c.type === 'add');
        const remove = changes.find(c => c.type === 'remove');

        expect(update).toMatchObject({ key: 'a', value: { name: 'Alice Updated' } });
        expect(add).toMatchObject({ key: 'c', value: { name: 'Charlie' } });
        expect(remove).toMatchObject({ key: 'b', previousValue: { name: 'Bob' } });
      });

      test('should not report changes when data is identical', () => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        handle.consumeChanges();

        // Same data again
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        expect(handle.getPendingChanges()).toHaveLength(0);
      });

      test('should include timestamp in changes', () => {
        const before = Date.now();
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
        const after = Date.now();

        const changes = handle.getPendingChanges();
        expect(changes[0].timestamp).toBeGreaterThanOrEqual(before);
        expect(changes[0].timestamp).toBeLessThanOrEqual(after);
      });
    });
  });
});

