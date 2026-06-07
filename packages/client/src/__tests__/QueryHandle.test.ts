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
      sort: { score: 'desc' },
    });

    const callback = jest.fn();
    handle.subscribe(callback);

    // Simulate receiving unsorted data from server
    handle.onResult(
      [
        { key: 'A', value: { id: 'A', score: 10 } },
        { key: 'B', value: { id: 'B', score: 30 } },
        { key: 'C', value: { id: 'C', score: 20 } },
      ],
      'server',
    );

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
      sort: { score: 'desc' },
    });

    const callback = jest.fn();
    handle.subscribe(callback);

    // Initial server response
    handle.onResult(
      [
        { key: 'A', value: { id: 'A', score: 10 } },
        { key: 'B', value: { id: 'B', score: 30 } },
      ],
      'server',
    );

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

    handle.onResult(
      [
        { key: 'node-123', value: { name: 'Node 1' } },
        { key: 'node-456', value: { name: 'Node 2' } },
      ],
      'server',
    );

    const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
    expect(lastCall[0]._key).toBe('node-123');
    expect(lastCall[0].name).toBe('Node 1');
    expect(lastCall[1]._key).toBe('node-456');
    expect(lastCall[1].name).toBe('Node 2');
  });

  describe('Settled latch', () => {
    test('should preserve local pre-load data until the first server response', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // Local pre-load data (loadInitialLocalData artifact, never settles)
      handle.onResult(
        [
          { key: 'A', value: { name: 'Local Item A' } },
          { key: 'B', value: { name: 'Local Item B' } },
        ],
        'local',
      );

      // No server response yet — query is NOT settled, local data stays visible
      expect(handle.isSettled).toBe(false);
      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(2);
      expect(lastCall[0]._key).toBe('A');
      expect(lastCall[1]._key).toBe('B');
    });

    test('settles on the FIRST server QUERY_RESP even when empty', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      handle.subscribe(jest.fn());

      expect(handle.isSettled).toBe(false);

      // Empty authoritative server response — settled = "a QUERY_RESP arrived",
      // not "rows arrived".
      handle.onResult([], 'server');

      expect(handle.isSettled).toBe(true);
    });

    test('settles on the FIRST server QUERY_RESP when non-empty', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      handle.subscribe(jest.fn());

      handle.onResult([{ key: 'A', value: { name: 'A' } }], 'server');

      expect(handle.isSettled).toBe(true);
    });

    test('local pre-load does NOT settle the query', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      handle.subscribe(jest.fn());

      handle.onResult([{ key: 'A', value: { name: 'A' } }], 'local');

      expect(handle.isSettled).toBe(false);
    });

    test('whenSettled() resolves when the first server QUERY_RESP arrives', async () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      handle.subscribe(jest.fn());

      let resolved = false;
      const settledPromise = handle.whenSettled().then(() => {
        resolved = true;
      });

      // Not settled yet
      await Promise.resolve();
      expect(resolved).toBe(false);

      handle.onResult([], 'server');

      await settledPromise;
      expect(resolved).toBe(true);
    });

    test('whenSettled() resolves immediately if already settled', async () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      handle.subscribe(jest.fn());

      handle.onResult([{ key: 'A', value: { name: 'A' } }], 'server');

      // Already settled — promise must resolve without further input
      await expect(handle.whenSettled()).resolves.toBeUndefined();
    });

    test('clears stale local-only rows on an empty server result once settled', () => {
      // AC3: offline/local writes, then an EMPTY authoritative server response.
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // Local-only rows (offline writes seeded into the cache)
      handle.onResult(
        [
          { key: 'local-1', value: { name: 'Local 1' } },
          { key: 'local-2', value: { name: 'Local 2' } },
        ],
        'local',
      );

      // Server authoritatively reports no rows for this query
      handle.onResult([], 'server');

      // Stale local-only rows are cleared and the latch is set even for the
      // empty set.
      expect(handle.isSettled).toBe(true);
      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(0);
    });

    test('should accept empty server response after receiving non-empty server data', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // First: server sends data (authoritative)
      handle.onResult([{ key: 'A', value: { name: 'Server Item A' } }], 'server');

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
      handle.onResult([{ key: 'local-only', value: { name: 'Local Only' } }], 'local');

      // Server sends different data
      handle.onResult([{ key: 'server-item', value: { name: 'Server Item' } }], 'server');

      // Server data should replace local
      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(1);
      expect(lastCall[0]._key).toBe('server-item');
    });

    test('server is authoritative: empty QUERY_RESP clears local-only rows', () => {
      // Client seeds a local row, then the server authoritatively reports none.
      // The server wins — local-only rows that the server does not return are
      // stale and get cleared.
      const handle = new QueryHandle<any>(mockSyncEngine, 'notes:user123', {});
      const callback = jest.fn();
      handle.subscribe(callback);

      // Client loads from IndexedDB
      handle.onResult([{ key: 'note1', value: { title: 'My Note', content: 'Content' } }], 'local');

      // Server responds empty (authoritative — no persisted rows for this query)
      handle.onResult([], 'server');

      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(0);
      expect(handle.isSettled).toBe(true);
    });
  });

  describe('subscribe { settled } meta argument', () => {
    test('AC5: local frame reports settled:false, server snapshot reports settled:true', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

      const seen: Array<{ keys: string[]; settled: boolean | undefined }> = [];
      handle.subscribe((results, meta) => {
        seen.push({ keys: results.map((r) => r._key), settled: meta?.settled });
      });

      // Local/optimistic frame — the server has NOT spoken yet.
      handle.onResult([{ key: 'A', value: { name: 'Local A' } }], 'local');

      // Server authoritative snapshot — settles the query.
      handle.onResult([{ key: 'A', value: { name: 'Server A' } }], 'server');

      // First emission is the local frame, unsettled.
      const localFrame = seen.find((s) => s.keys.length === 1 && s.settled === false);
      expect(localFrame).toBeDefined();

      // A later emission carries settled:true after the server responds.
      const settledFrame = seen.find((s) => s.settled === true);
      expect(settledFrame).toBeDefined();

      // The terminal emission must be settled.
      expect(seen[seen.length - 1].settled).toBe(true);
    });

    test('settled stays false across multiple local-only frames', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

      const settledValues: Array<boolean | undefined> = [];
      handle.subscribe((_results, meta) => settledValues.push(meta?.settled));

      handle.onResult([{ key: 'A', value: { name: 'A' } }], 'local');
      handle.onUpdate('A', { name: 'A2' });

      expect(settledValues.every((v) => v === false)).toBe(true);
    });

    test('an empty server response still flips meta.settled to true', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

      let lastSettled: boolean | undefined;
      handle.subscribe((_results, meta) => {
        lastSettled = meta?.settled;
      });

      handle.onResult([], 'server');

      expect(lastSettled).toBe(true);
    });

    test('back-compat: a single-arg (results) => void subscriber still receives results', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

      // Deliberately single-arg, exactly as React useQuery calls it.
      const singleArg = jest.fn((results: any) => results);
      handle.subscribe(singleArg);

      handle.onResult([{ key: 'A', value: { name: 'Alice' } }], 'server');

      expect(singleArg).toHaveBeenCalled();
      const lastCall = singleArg.mock.calls[singleArg.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(1);
      expect(lastCall[0]._key).toBe('A');
    });

    test('a late subscriber receives cached results with the current settled state', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

      // First subscriber activates the query.
      handle.subscribe(jest.fn());
      handle.onResult([{ key: 'A', value: { name: 'Alice' } }], 'server');

      // A second subscriber gets the immediate cached invocation.
      const late = jest.fn();
      handle.subscribe(late);

      expect(late).toHaveBeenCalledTimes(1);
      const [results, meta] = late.mock.calls[0];
      expect(results).toHaveLength(1);
      expect(meta).toEqual({ settled: true });
    });
  });

  describe('Subscriber isolation in notify()', () => {
    test('AC4: a throwing subscriber does not block later subscribers or propagate', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

      const throwingSubscriber = jest.fn(() => {
        throw new Error('subscriber boom');
      });
      const normalSubscriber = jest.fn();

      handle.subscribe(throwingSubscriber);
      handle.subscribe(normalSubscriber);

      // Deliver a result emission — must not throw out of onResult.
      expect(() => {
        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');
      }).not.toThrow();

      expect(throwingSubscriber).toHaveBeenCalled();
      // Normal subscriber still receives the result despite the earlier throw.
      expect(normalSubscriber).toHaveBeenCalled();
      const lastCall = normalSubscriber.mock.calls[normalSubscriber.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(1);
      expect(lastCall[0]._key).toBe('a');
    });

    test('a throwing subscriber does not propagate out of onUpdate', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'items', {});

      // First subscriber throws; subscribing it first avoids the immediate
      // cached-result invocation (only later subscribers get that), so the
      // throw is exercised exclusively through the notify() path on onUpdate.
      const throwingSubscriber = jest.fn(() => {
        throw new Error('subscriber boom');
      });
      handle.subscribe(throwingSubscriber);
      handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

      expect(() => {
        handle.onUpdate('a', { name: 'Alice Updated' });
      }).not.toThrow();
      expect(throwingSubscriber).toHaveBeenCalled();
    });
  });

  describe('Change tracking', () => {
    let handle: QueryHandle<{ name: string }>;

    beforeEach(() => {
      handle = new QueryHandle<{ name: string }>(mockSyncEngine, 'items', {});
      handle.subscribe(() => {}); // Activate handle
    });

    describe('onDelta', () => {
      test('should subscribe to change events', () => {
        const changeListener = jest.fn();
        const unsubscribe = handle.onDelta(changeListener);

        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        expect(changeListener).toHaveBeenCalledTimes(1);
        expect(changeListener).toHaveBeenCalledWith(
          expect.arrayContaining([
            expect.objectContaining({ type: 'add', key: 'a', value: { name: 'Alice' } }),
          ]),
        );

        unsubscribe();
      });

      test('should unsubscribe from change events', () => {
        const changeListener = jest.fn();
        const unsubscribe = handle.onDelta(changeListener);

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

        handle.onDelta(listener1);
        handle.onDelta(listener2);

        handle.onResult([{ key: 'a', value: { name: 'Alice' } }], 'server');

        expect(listener1).toHaveBeenCalledTimes(1);
        expect(listener2).toHaveBeenCalledTimes(1);
      });

      test('should catch errors in change listeners without affecting others', () => {
        const errorListener = jest.fn(() => {
          throw new Error('Test error');
        });
        const normalListener = jest.fn();

        handle.onDelta(errorListener);
        handle.onDelta(normalListener);

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
        handle.onDelta(listener);

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
          previousValue: { name: 'Alice' },
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
        expect(pending1).toEqual(pending2); // Same content
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
          value: { name: 'Alice' },
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
          previousValue: { name: 'Alice' },
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
          previousValue: { name: 'Alice' },
        });
        expect(changes[0].value).toBeUndefined();
      });

      test('should detect mixed changes in batch', () => {
        handle.onResult(
          [
            { key: 'a', value: { name: 'Alice' } },
            { key: 'b', value: { name: 'Bob' } },
          ],
          'server',
        );
        handle.consumeChanges();

        // a: updated, b: removed, c: added
        handle.onResult(
          [
            { key: 'a', value: { name: 'Alice Updated' } },
            { key: 'c', value: { name: 'Charlie' } },
          ],
          'server',
        );

        const changes = handle.getPendingChanges();
        expect(changes).toHaveLength(3);

        const update = changes.find((c) => c.type === 'update');
        const add = changes.find((c) => c.type === 'add');
        const remove = changes.find((c) => c.type === 'remove');

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
