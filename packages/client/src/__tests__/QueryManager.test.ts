import { QueryManager } from '../sync/QueryManager';
import type { QueryManagerConfig } from '../sync/types';
import type { IStorageAdapter } from '../IStorageAdapter';
import { QueryHandle } from '../QueryHandle';
import { HybridQueryHandle } from '../HybridQueryHandle';
import { SyncEngine } from '../SyncEngine';

// Mock SyncEngine for QueryHandle/HybridQueryHandle
const mockSyncEngine = {
  subscribeToQuery: jest.fn(),
  unsubscribeFromQuery: jest.fn(),
  runLocalQuery: jest.fn().mockResolvedValue([]),
  subscribeToHybridQuery: jest.fn(),
  unsubscribeFromHybridQuery: jest.fn(),
  runLocalHybridQuery: jest.fn().mockResolvedValue([]),
} as unknown as SyncEngine;

// --- Mock Storage Adapter ---
function createMockStorageAdapter(): jest.Mocked<IStorageAdapter> {
  return {
    initialize: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(undefined),
    put: jest.fn().mockResolvedValue(undefined),
    remove: jest.fn().mockResolvedValue(undefined),
    getMeta: jest.fn().mockResolvedValue(undefined),
    setMeta: jest.fn().mockResolvedValue(undefined),
    batchPut: jest.fn().mockResolvedValue(undefined),
    appendOpLog: jest.fn().mockResolvedValue(1),
    getPendingOps: jest.fn().mockResolvedValue([]),
    markOpsSynced: jest.fn().mockResolvedValue(undefined),
    deleteOp: jest.fn().mockResolvedValue(undefined),
    commitWrite: jest.fn().mockResolvedValue(1),
    getAllKeys: jest.fn().mockResolvedValue([]),
    getAllMetaKeys: jest.fn().mockResolvedValue([]),
  };
}

describe('QueryManager', () => {
  let queryManager: QueryManager;
  let mockStorage: jest.Mocked<IStorageAdapter>;
  let mockSendMessage: jest.Mock;
  let mockIsAuthenticated: jest.Mock;
  let config: QueryManagerConfig;

  beforeEach(() => {
    mockStorage = createMockStorageAdapter();
    mockSendMessage = jest.fn().mockReturnValue(true);
    mockIsAuthenticated = jest.fn().mockReturnValue(false);

    config = {
      storageAdapter: mockStorage,
      sendMessage: mockSendMessage,
      isAuthenticated: mockIsAuthenticated,
    };

    queryManager = new QueryManager(config);
  });

  describe('Standard Queries', () => {
    it('should subscribe to a query and add it to the map', () => {
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});

      queryManager.subscribeToQuery(query);

      expect(queryManager.getQueries().has(query.id)).toBe(true);
    });

    it('should send subscription message when authenticated', () => {
      mockIsAuthenticated.mockReturnValue(true);
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {
        where: { status: 'active' },
      });

      queryManager.subscribeToQuery(query);

      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_SUB',
        payload: {
          queryId: query.id,
          mapName: 'test-map',
          query: { where: { status: 'active' } },
        },
      });
    });

    it('should serialize sort as ordered array on the wire', () => {
      mockIsAuthenticated.mockReturnValue(true);
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {
        where: { status: 'active' },
        sort: { createdAt: 'desc', name: 'asc' },
      });

      queryManager.subscribeToQuery(query);

      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_SUB',
        payload: {
          queryId: query.id,
          mapName: 'test-map',
          // sort is serialized as ordered array, not a plain object
          query: {
            where: { status: 'active' },
            sort: [
              { field: 'createdAt', direction: 'desc' },
              { field: 'name', direction: 'asc' },
            ],
          },
        },
      });
    });

    it('should not send subscription message when not authenticated', () => {
      mockIsAuthenticated.mockReturnValue(false);
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});

      queryManager.subscribeToQuery(query);

      expect(mockSendMessage).not.toHaveBeenCalled();
    });

    it('should unsubscribe from a query and remove it from the map', () => {
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToQuery(query);

      queryManager.unsubscribeFromQuery(query.id);

      expect(queryManager.getQueries().has(query.id)).toBe(false);
    });

    it('should send unsubscription message when authenticated', () => {
      mockIsAuthenticated.mockReturnValue(true);
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToQuery(query);
      mockSendMessage.mockClear();

      queryManager.unsubscribeFromQuery(query.id);

      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_UNSUB',
        payload: { queryId: query.id },
      });
    });
  });

  describe('Hybrid Queries', () => {
    it('should subscribe to a hybrid query and add it to the map', () => {
      const query = new HybridQueryHandle<any>(mockSyncEngine, 'test-map', {});

      queryManager.subscribeToHybridQuery(query);

      expect(queryManager.getHybridQueries().has(query.id)).toBe(true);
    });

    it('should get a hybrid query by ID', () => {
      const query = new HybridQueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToHybridQuery(query);

      const result = queryManager.getHybridQuery(query.id);

      expect(result).toBe(query);
    });

    it('should unsubscribe from a hybrid query and remove it from the map', () => {
      const query = new HybridQueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToHybridQuery(query);

      queryManager.unsubscribeFromHybridQuery(query.id);

      expect(queryManager.getHybridQueries().has(query.id)).toBe(false);
    });
  });

  describe('runLocalQuery', () => {
    it('should return matching records from storage', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['users:1', 'users:2', 'users:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'users:1': { value: { name: 'Alice', status: 'active' } },
          'users:2': { value: { name: 'Bob', status: 'inactive' } },
          'users:3': { value: { name: 'Charlie', status: 'active' } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalQuery('users', { where: { status: 'active' } });

      expect(results).toHaveLength(2);
      expect(results[0].key).toBe('1');
      expect(results[0].value.name).toBe('Alice');
      expect(results[1].key).toBe('3');
      expect(results[1].value.name).toBe('Charlie');
    });

    it('should return empty array when no records match', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['users:1']);
      mockStorage.get.mockResolvedValue({ value: { status: 'inactive' } });

      const results = await queryManager.runLocalQuery('users', { where: { status: 'active' } });

      expect(results).toHaveLength(0);
    });

    it('should filter by predicate', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['items:1', 'items:2', 'items:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'items:1': { value: { price: 50 } },
          'items:2': { value: { price: 150 } },
          'items:3': { value: { price: 200 } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalQuery('items', {
        predicate: { op: 'gt', attribute: 'price', value: 100 },
      });

      expect(results).toHaveLength(2);
      expect(results[0].value.price).toBe(150);
      expect(results[1].value.price).toBe(200);
    });

    // F11 (TODO-501): offline parity — the local engine used to ignore
    // sort/limit and silently dropped cursor/groupBy/aggregations, so offline
    // results diverged from the server's.
    describe('offline parity: sort / limit / unsupported clauses (F11)', () => {
      beforeEach(() => {
        mockStorage.getAllKeys.mockResolvedValue(['items:1', 'items:2', 'items:3']);
        mockStorage.get.mockImplementation(async (key: string) => {
          const data: Record<string, any> = {
            'items:1': { value: { name: 'b', price: 20 } },
            'items:2': { value: { name: 'a', price: 30 } },
            'items:3': { value: { name: 'c', price: 10 } },
          };
          return data[key];
        });
      });

      it('applies sort offline (ascending) instead of returning storage order', async () => {
        const results = await queryManager.runLocalQuery('items', { sort: { price: 'asc' } });
        expect(results.map((r) => r.value.price)).toEqual([10, 20, 30]);
      });

      it('applies sort offline (descending)', async () => {
        const results = await queryManager.runLocalQuery('items', { sort: { name: 'desc' } });
        expect(results.map((r) => r.value.name)).toEqual(['c', 'b', 'a']);
      });

      it('clamps to limit offline after sorting (server-shaped top-N)', async () => {
        const results = await queryManager.runLocalQuery('items', {
          sort: { price: 'asc' },
          limit: 2,
        });
        expect(results.map((r) => r.value.price)).toEqual([10, 20]);
      });

      it('rejects groupBy offline instead of silently ignoring it', async () => {
        await expect(queryManager.runLocalQuery('items', { groupBy: ['name'] })).rejects.toThrow(
          /offline query does not support groupBy/i,
        );
      });

      it('rejects aggregations offline', async () => {
        await expect(
          queryManager.runLocalQuery('items', {
            aggregations: [{ func: 'sum', field: 'price' } as any],
          }),
        ).rejects.toThrow(/aggregations/i);
      });

      it('rejects cursor offline', async () => {
        await expect(queryManager.runLocalQuery('items', { cursor: 'abc' })).rejects.toThrow(
          /cursor/i,
        );
      });
    });
  });

  describe('runLocalHybridQuery', () => {
    it('should return matching records with score metadata', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['docs:1', 'docs:2']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'docs:1': { value: { title: 'Hello World', category: 'tech' } },
          'docs:2': { value: { title: 'Goodbye', category: 'tech' } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalHybridQuery<any>('docs', {
        where: { category: 'tech' },
      });

      expect(results).toHaveLength(2);
      expect(results[0].score).toBe(0); // Local queries have score 0
      expect(results[0].matchedTerms).toEqual([]);
    });

    it('should apply sorting', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['items:1', 'items:2', 'items:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'items:1': { value: { name: 'C', price: 30 } },
          'items:2': { value: { name: 'A', price: 10 } },
          'items:3': { value: { name: 'B', price: 20 } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalHybridQuery<any>('items', {
        sort: { name: 'asc' },
      });

      expect(results[0].value.name).toBe('A');
      expect(results[1].value.name).toBe('B');
      expect(results[2].value.name).toBe('C');
    });

    it('should apply limit', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['items:1', 'items:2', 'items:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'items:1': { value: { name: 'A' } },
          'items:2': { value: { name: 'B' } },
          'items:3': { value: { name: 'C' } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalHybridQuery<any>('items', {
        limit: 2,
      });

      expect(results).toHaveLength(2);
    });
  });

  describe('resubscribeAll', () => {
    it('should resubscribe all standard queries', () => {
      mockIsAuthenticated.mockReturnValue(true);
      const query1 = new QueryHandle<any>(mockSyncEngine, 'map1', { where: { a: 1 } });
      const query2 = new QueryHandle<any>(mockSyncEngine, 'map2', { where: { b: 2 } });

      // Subscribe while authenticated
      queryManager.subscribeToQuery(query1);
      queryManager.subscribeToQuery(query2);
      mockSendMessage.mockClear();

      // Resubscribe
      queryManager.resubscribeAll();

      expect(mockSendMessage).toHaveBeenCalledTimes(2);
      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_SUB',
        payload: expect.objectContaining({ queryId: query1.id }),
      });
      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_SUB',
        payload: expect.objectContaining({ queryId: query2.id }),
      });
    });

    it('should resubscribe hybrid queries with FTS predicates', () => {
      mockIsAuthenticated.mockReturnValue(true);

      // Create a hybrid query with FTS predicate
      const hybridQuery = new HybridQueryHandle<any>(mockSyncEngine, 'docs', {
        predicate: { op: 'match', attribute: 'content', query: 'search term' },
      });

      queryManager.subscribeToHybridQuery(hybridQuery);
      mockSendMessage.mockClear();

      queryManager.resubscribeAll();

      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'HYBRID_QUERY_SUBSCRIBE',
        payload: expect.objectContaining({ subscriptionId: hybridQuery.id }),
      });
    });
  });
});

describe('QueryManager error QUERY_RESP routing', () => {
  const OVERFLOW = 'QUERY_SNAPSHOT_OVERFLOW';
  const REFUSAL = 'QUERY_LIVE_AGGREGATE_UNSUPPORTED';
  const ROWS = [
    { key: 'a', value: { n: 1 } },
    { key: 'b', value: { n: 2 } },
  ];

  let manager: QueryManager;
  let sent: jest.Mock;
  let authenticated: boolean;
  let random: jest.SpyInstance;

  beforeEach(() => {
    jest.useFakeTimers();
    random = jest.spyOn(Math, 'random').mockReturnValue(1);
    sent = jest.fn(() => true);
    authenticated = true;
    manager = new QueryManager({
      storageAdapter: createMockStorageAdapter(),
      sendMessage: sent,
      isAuthenticated: () => authenticated,
    });
  });

  afterEach(() => {
    random.mockRestore();
    jest.useRealTimers();
  });

  /** A live handle subscribed through the manager that already shows ROWS. */
  function liveHandleWithRows(options?: { snapshotOnly: boolean }): QueryHandle<any> {
    const handle = new QueryHandle<any>(mockSyncEngine, 'm', {}, options);
    manager.subscribeToQuery(handle);
    manager.handleQueryResp({
      queryId: handle.id,
      results: ROWS,
      nextCursor: 'c1',
      hasMore: true,
      cursorStatus: 'valid',
    });
    return handle;
  }

  function querySubs(handle: QueryHandle<any>): number {
    return sent.mock.calls.filter(
      ([message]) => message.type === 'QUERY_SUB' && message.payload.queryId === handle.id,
    ).length;
  }

  function overflow(handle: QueryHandle<any>): void {
    manager.handleQueryResp({
      queryId: handle.id,
      results: [],
      error: 'buffer overflowed',
      code: OVERFLOW,
    });
  }

  function refuse(handle: QueryHandle<any>, code: string = REFUSAL): void {
    manager.handleQueryResp({ queryId: handle.id, results: [], error: 'refused', code });
  }

  function clean(handle: QueryHandle<any>): void {
    manager.handleQueryResp({ queryId: handle.id, results: ROWS });
  }

  /** Overflow until the bound runs out; the handle ends with the overflow error. */
  function exhaust(handle: QueryHandle<any>): void {
    for (let i = 0; i < 3; i++) {
      overflow(handle);
      jest.advanceTimersByTime(4000);
    }
    overflow(handle);
    expect(handle.getError()?.code).toBe(OVERFLOW);
  }

  function keys(handle: QueryHandle<any>): string[] {
    return handle.getSnapshot().map((row) => row._key);
  }

  describe('an error frame never touches rows', () => {
    test.each([
      ['with rows in the frame', [{ key: 'z', value: { n: 9 } }]],
      ['with no rows in the frame', []],
    ])('%s', (_label, results) => {
      const handle = liveHandleWithRows();
      const callback = jest.fn();
      const deltas = jest.fn();
      handle.subscribe(callback);
      handle.onDelta(deltas);
      callback.mockClear();
      const pagination = handle.getPaginationInfo();

      for (const code of [OVERFLOW, REFUSAL]) {
        manager.handleQueryResp({ queryId: handle.id, results, error: 'e', code });
      }

      expect(callback).not.toHaveBeenCalled();
      expect(deltas).not.toHaveBeenCalled();
      expect(handle.getPaginationInfo()).toEqual(pagination);
      expect(keys(handle)).toEqual(['a', 'b']);
    });

    test('a frame for an unknown query id is ignored', () => {
      expect(() =>
        manager.handleQueryResp({ queryId: 'nope', results: [], error: 'e', code: OVERFLOW }),
      ).not.toThrow();
      expect(sent).not.toHaveBeenCalled();
    });
  });

  describe('a snapshot overflow resubscribes with a bound', () => {
    test('overflows 1-3 each resubscribe after 250/1000/4000 ms; the 4th raises the error', () => {
      const handle = liveHandleWithRows();
      expect(querySubs(handle)).toBe(1);
      const delays = [250, 1000, 4000];
      for (const [i, delay] of delays.entries()) {
        overflow(handle);
        jest.advanceTimersByTime(delay - 1);
        expect(querySubs(handle)).toBe(1 + i);
        jest.advanceTimersByTime(1);
        expect(querySubs(handle)).toBe(2 + i);
        const resub = sent.mock.calls[sent.mock.calls.length - 1][0];
        expect(resub.payload.queryId).toBe(handle.id);
        expect(handle.getError()).toBeNull();
      }
      overflow(handle);
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(4);
      expect(handle.getError()?.code).toBe(OVERFLOW);
      expect(handle.getError()?.message).toContain(OVERFLOW);
      expect(keys(handle)).toEqual(['a', 'b']);
    });

    test('the delay is scaled by full jitter in [0.5, 1]', () => {
      random.mockReturnValue(0);
      const handle = liveHandleWithRows();
      overflow(handle);
      jest.advanceTimersByTime(124);
      expect(querySubs(handle)).toBe(1);
      jest.advanceTimersByTime(1);
      expect(querySubs(handle)).toBe(2);
    });

    test('a timer that fires while unauthenticated sends nothing', () => {
      const handle = liveHandleWithRows();
      overflow(handle);
      authenticated = false;
      jest.advanceTimersByTime(10_000);
      expect(querySubs(handle)).toBe(1);
    });

    test('an overflow while a timer is pending schedules nothing more; the pending one still sends', () => {
      const handle = liveHandleWithRows();
      overflow(handle);
      overflow(handle);
      jest.advanceTimersByTime(250);
      expect(querySubs(handle)).toBe(2);
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(2);
      expect(handle.getError()).toBeNull();
    });

    test('an overflow that finds the count at 3 cancels a pending timer and raises at once', () => {
      const handle = liveHandleWithRows();
      overflow(handle);
      jest.advanceTimersByTime(250);
      overflow(handle);
      jest.advanceTimersByTime(1000);
      overflow(handle); // third: its timer is pending
      expect(querySubs(handle)).toBe(3);
      overflow(handle); // fourth: count is already 3
      expect(handle.getError()?.code).toBe(OVERFLOW);
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(3);
    });
  });

  describe('every other code is terminal', () => {
    test('QUERY_LIVE_AGGREGATE_UNSUPPORTED raises its code and never resubscribes', () => {
      const handle = liveHandleWithRows();
      refuse(handle);
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(1);
      expect(handle.getError()?.code).toBe(REFUSAL);
    });

    test('an unknown code becomes QUERY_FAILED with the raw code in the message', () => {
      const handle = liveHandleWithRows();
      refuse(handle, 'X_NEW_CODE');
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(1);
      expect(handle.getError()?.code).toBe('QUERY_FAILED');
      expect(handle.getError()?.message).toContain('X_NEW_CODE');
    });

    test('an error without a code becomes QUERY_FAILED', () => {
      const handle = liveHandleWithRows();
      manager.handleQueryResp({ queryId: handle.id, results: [], error: 'refused' });
      expect(handle.getError()?.code).toBe('QUERY_FAILED');
      expect(handle.getError()?.message).toBe('QUERY_FAILED: refused');
    });

    test('an overflow on a one-shot handle is terminal', () => {
      const handle = new QueryHandle<any>(mockSyncEngine, 'm', {}, { snapshotOnly: true });
      manager.subscribeToQuery(handle);
      overflow(handle);
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(1);
      expect(handle.getError()?.code).toBe(OVERFLOW);
      // Terminal kind: a chain start does not clear it, a clean answer in that later chain does.
      manager.resubscribeAll();
      expect(handle.getError()?.code).toBe(OVERFLOW);
      clean(handle);
      expect(handle.getError()).toBeNull();
    });
  });

  describe("the error's lifetime", () => {
    test('overflow error: resubscribeAll clears it and the next overflow resubscribes again', () => {
      const handle = liveHandleWithRows();
      exhaust(handle);
      const exhausted = handle.getError();
      const seen: unknown[] = [];
      handle.onError((error) => seen.push(error));
      manager.resubscribeAll();
      expect(seen).toEqual([exhausted, null]);
      const before = querySubs(handle);
      overflow(handle);
      jest.advanceTimersByTime(250);
      expect(querySubs(handle)).toBe(before + 1);
    });

    test('overflow error: a remount (every listener left, a new one subscribes) clears it', () => {
      const handle = liveHandleWithRows();
      exhaust(handle);
      manager.unsubscribeFromQuery(handle.id);
      manager.subscribeToQuery(handle);
      expect(handle.getError()).toBeNull();
      const before = querySubs(handle);
      overflow(handle);
      jest.advanceTimersByTime(249);
      expect(querySubs(handle)).toBe(before);
      jest.advanceTimersByTime(1);
      expect(querySubs(handle)).toBe(before + 1);
    });

    test('overflow error: the snapshot that follows the exhausting overflow does not clear it', () => {
      const handle = liveHandleWithRows();
      exhaust(handle);
      clean(handle);
      expect(handle.getError()?.code).toBe(OVERFLOW);
    });

    test('terminal error: a reconnect keeps it; a clean answer in the new chain clears it', () => {
      const handle = liveHandleWithRows();
      refuse(handle);
      const seen: unknown[] = [];
      handle.onError((error) => seen.push(error));
      seen.length = 0;
      manager.resubscribeAll();
      expect(handle.getError()?.code).toBe(REFUSAL);
      expect(seen).toEqual([]);
      clean(handle);
      expect(handle.getError()).toBeNull();
      expect(seen).toEqual([null]);
    });

    test('terminal error: a clean answer in the chain that raised it does not clear it', () => {
      const handle = liveHandleWithRows();
      refuse(handle);
      clean(handle);
      expect(handle.getError()?.code).toBe(REFUSAL);
    });

    test('unsubscribeFromQuery with a pending timer sends no later QUERY_SUB', () => {
      const handle = liveHandleWithRows();
      overflow(handle);
      manager.unsubscribeFromQuery(handle.id);
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(1);
    });

    test('close() leaves no pending timer', () => {
      const handle = liveHandleWithRows();
      overflow(handle);
      expect(jest.getTimerCount()).toBe(1);
      manager.close();
      expect(jest.getTimerCount()).toBe(0);
    });

    test('resubscribeAll with a pending timer: only the new chain sends a QUERY_SUB', () => {
      const handle = liveHandleWithRows();
      overflow(handle);
      const before = querySubs(handle);
      manager.resubscribeAll();
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(before + 1);
    });

    test('unsubscribe then subscribeToQuery with a pending timer: only the new chain sends a QUERY_SUB', () => {
      const handle = liveHandleWithRows();
      overflow(handle);
      manager.unsubscribeFromQuery(handle.id);
      const before = querySubs(handle);
      manager.subscribeToQuery(handle);
      jest.advanceTimersByTime(60_000);
      expect(querySubs(handle)).toBe(before + 1);
    });
  });

  describe('whenSettled after an error clears before settlement', () => {
    function unsettledHandle(): QueryHandle<any> {
      const handle = new QueryHandle<any>(mockSyncEngine, 'm', {});
      manager.subscribeToQuery(handle);
      return handle;
    }

    test('overflow error: the next chain gives new awaiters a promise its snapshot resolves', async () => {
      const handle = unsettledHandle();
      exhaust(handle);
      await expect(handle.whenSettled()).rejects.toMatchObject({ code: OVERFLOW });
      manager.resubscribeAll();
      const settled = handle.whenSettled();
      clean(handle);
      await expect(settled).resolves.toBeUndefined();
      expect(handle.isSettled).toBe(true);
    });

    test('terminal error: the clean answer of a later chain resolves whenSettled', async () => {
      const handle = unsettledHandle();
      refuse(handle);
      await expect(handle.whenSettled()).rejects.toMatchObject({ code: REFUSAL });
      manager.resubscribeAll();
      clean(handle);
      expect(handle.getError()).toBeNull();
      await expect(handle.whenSettled()).resolves.toBeUndefined();
    });
  });
});
