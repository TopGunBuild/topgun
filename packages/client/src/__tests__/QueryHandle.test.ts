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
});

