import { TopGunClient } from '../TopGunClient';
import { QueryHandle } from '../QueryHandle';
import { SyncState } from '../SyncState';
import { QueryOnceUnsettledError, QueryOnceLocalError } from '../errors/QueryOnceError';
import { QueryError } from '../errors/QueryError';
import { QueryManager } from '../sync/QueryManager';
import type { QueryOncePagedResult } from '../TopGunClient';
import type { QueryFilter } from '../QueryHandle';
import type { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';

// crypto.randomUUID is needed by the TopGunClient constructor in Node test envs.
if (!(global as { crypto?: { randomUUID?: () => string } }).crypto?.randomUUID) {
  let n = 0;
  Object.defineProperty(global, 'crypto', {
    configurable: true,
    value: { randomUUID: () => `qo-uuid-${++n}` },
  });
}

/** Minimal in-memory storage adapter (subset of the shared TopGunClient.test one). */
class MemoryStorageAdapter implements IStorageAdapter {
  private kv = new Map<string, unknown>();
  private meta = new Map<string, unknown>();
  private opLog: OpLogEntry[] = [];
  private pending: OpLogEntry[] = [];

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  async get<V>(key: string): Promise<V | undefined> {
    return this.kv.get(key) as V | undefined;
  }
  async put(key: string, value: unknown): Promise<void> {
    this.kv.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.kv.delete(key);
  }
  async getMeta(key: string): Promise<unknown> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: unknown): Promise<void> {
    this.meta.set(key, value);
  }
  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [k, v] of entries) this.kv.set(k, v);
  }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opLog.length + 1;
    const e = { ...entry, id, synced: 0 } as OpLogEntry;
    this.opLog.push(e);
    this.pending.push(e);
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.pending;
  }
  async markOpsSynced(lastId: number): Promise<void> {
    this.pending = this.pending.filter((op) => (op.id ?? 0) > lastId);
    this.opLog = this.opLog.filter((op) => (op.id ?? 0) > lastId);
  }
  async deleteOp(id: number): Promise<void> {
    this.pending = this.pending.filter((op) => op.id !== id);
    this.opLog = this.opLog.filter((op) => op.id !== id);
  }
  async commitWrite(
    mutations: Array<{
      store: 'kv' | 'meta';
      type: 'put' | 'remove';
      key: string;
      value?: unknown;
    }>,
    op: Omit<OpLogEntry, 'id'>,
  ): Promise<number> {
    for (const m of mutations) {
      const target = m.store === 'meta' ? this.meta : this.kv;
      if (m.type === 'remove') target.delete(m.key);
      else target.set(m.key, m.value);
    }
    return this.appendOpLog(op);
  }
  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kv.keys());
  }
  async getAllMetaKeys(): Promise<string[]> {
    return Array.from(this.meta.keys());
  }
}

/**
 * Behavioral coverage for TopGunClient.queryOnce.
 *
 * queryOnce constructs its own QueryHandle internally, so to simulate a server
 * QUERY_RESP we intercept the SyncEngine the client uses: subscribeToQuery hands
 * us the live handle, and we drive handle.onResult(..., 'server') on it — exactly
 * the QUERY_RESP path the real SyncEngine uses (see QueryHandle.test.ts harness).
 *
 * We replace the client's private syncEngine with a controllable double after
 * construction. This keeps the test focused on queryOnce's settle/offline logic
 * without booting a real WebSocket server.
 */

interface SyncEngineDouble {
  setState(state: SyncState): void;
  // The handle most recently passed to subscribeToQuery (queryOnce's handle).
  lastHandle(): QueryHandle<unknown> | undefined;
  subscribeToQuery: jest.Mock;
  unsubscribeFromQuery: jest.Mock;
  runLocalQuery: jest.Mock;
  getConnectionState: jest.Mock;
}

function makeSyncEngineDouble(initialState: SyncState): SyncEngineDouble {
  let state = initialState;
  let captured: QueryHandle<unknown> | undefined;

  const subscribeToQuery = jest.fn((handle: QueryHandle<unknown>) => {
    captured = handle;
  });

  return {
    setState(s: SyncState) {
      state = s;
    },
    lastHandle() {
      return captured;
    },
    subscribeToQuery,
    unsubscribeFromQuery: jest.fn(),
    // Local snapshot the QueryHandle pre-loads (loadInitialLocalData).
    runLocalQuery: jest.fn().mockResolvedValue([]),
    getConnectionState: jest.fn(() => state),
  };
}

/** Build a client and swap in the controllable SyncEngine double. */
function makeClient(state: SyncState): {
  client: TopGunClient;
  engine: SyncEngineDouble;
} {
  // Local-only mode (no serverUrl) so construction never opens a real socket; the
  // real SyncEngine is discarded immediately and replaced by the controllable double.
  const client = new TopGunClient({
    storage: new MemoryStorageAdapter(),
  });
  const engine = makeSyncEngineDouble(state);
  (client as unknown as { syncEngine: SyncEngineDouble }).syncEngine = engine;
  return { client, engine };
}

/** Push an authoritative server QUERY_RESP into the active queryOnce handle. */
async function settleServer(
  engine: SyncEngineDouble,
  items: { key: string; value: unknown }[],
): Promise<void> {
  // subscribeToQuery is invoked synchronously inside subscribe(); flush a microtask
  // so the handle is captured before we drive onResult.
  await Promise.resolve();
  const handle = engine.lastHandle();
  if (!handle) throw new Error('queryOnce did not subscribe a handle');
  handle.onResult(items, 'server');
}

/** Push a QUERY_RESP with pagination metadata into the active queryOncePaged handle. */
async function settleServerPaged(
  engine: SyncEngineDouble,
  items: { key: string; value: unknown }[],
  pagination: { nextCursor?: string; hasMore: boolean },
): Promise<void> {
  await Promise.resolve();
  const handle = engine.lastHandle();
  if (!handle) throw new Error('queryOncePaged did not subscribe a handle');
  handle.onResult(items, 'server');
  handle.updatePaginationInfo({
    nextCursor: pagination.nextCursor,
    hasMore: pagination.hasMore,
    cursorStatus: pagination.nextCursor ? 'valid' : 'none',
  });
}

describe('TopGunClient.queryOnce', () => {
  describe('AC1 — returns authoritative server data not local []', () => {
    test('resolves with a server-only record (not [])', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);
      // Local store has nothing for this query — record exists ONLY on the server.
      engine.runLocalQuery.mockResolvedValue([]);

      const promise = client.queryOnce('users', {});

      await settleServer(engine, [{ key: 'u1', value: { id: 'u1', name: 'Ada' } }]);

      const results = await promise;
      expect(results).toHaveLength(1);
      expect(results[0]._key).toBe('u1');
      expect((results[0] as { name: string }).name).toBe('Ada');
    });

    test('resolves with an empty authoritative result when the server has no rows', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);
      const promise = client.queryOnce('users', {});

      // Empty server QUERY_RESP still settles — "the server has no rows" is a real answer.
      await settleServer(engine, []);

      const results = await promise;
      expect(results).toEqual([]);
    });

    test('auto-unsubscribes after resolving (no live subscription leak)', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);
      const promise = client.queryOnce('users', {});
      await settleServer(engine, [{ key: 'u1', value: { id: 'u1' } }]);
      await promise;

      expect(engine.unsubscribeFromQuery).toHaveBeenCalledTimes(1);
    });
  });

  describe('AC2 — explicit offline policy, never silently stale', () => {
    test('default offline → REJECTS with QueryOnceUnsettledError (offline)', async () => {
      const { client } = makeClient(SyncState.DISCONNECTED);

      await expect(client.queryOnce('users', {})).rejects.toBeInstanceOf(QueryOnceUnsettledError);
      await expect(client.queryOnce('users', {})).rejects.toMatchObject({
        code: 'QUERY_ONCE_UNSETTLED',
        reason: 'offline',
      });
    });

    test('default offline does NOT return a local snapshot', async () => {
      const { client, engine } = makeClient(SyncState.DISCONNECTED);
      // Even with local data present, default policy must reject — never stale.
      engine.runLocalQuery.mockResolvedValue([
        { key: 'u1', value: { id: 'u1', name: 'StaleLocal' } },
      ]);

      await expect(client.queryOnce('users', {})).rejects.toBeInstanceOf(QueryOnceUnsettledError);
    });

    test('default timeout → REJECTS with QueryOnceUnsettledError (timeout)', async () => {
      const { client } = makeClient(SyncState.CONNECTED);
      // Online but server never settles within the window.
      await expect(client.queryOnce('users', {}, { timeoutMs: 20 })).rejects.toMatchObject({
        code: 'QUERY_ONCE_UNSETTLED',
        reason: 'timeout',
      });
    });

    test('allowLocal offline → throws QueryOnceLocalError carrying the local snapshot', async () => {
      const { client, engine } = makeClient(SyncState.DISCONNECTED);
      engine.runLocalQuery.mockResolvedValue([
        { key: 'u1', value: { id: 'u1', name: 'LocalAda' } },
      ]);

      const promise = client.queryOnce('users', {}, { allowLocal: true });
      // Let the handle's loadInitialLocalData microtasks seed `latest` before reject.
      await Promise.resolve();
      await Promise.resolve();

      await expect(promise).rejects.toBeInstanceOf(QueryOnceLocalError);
    });

    test('allowLocal offline → caller can read err.localData (non-settled, distinguishable)', async () => {
      const { client, engine } = makeClient(SyncState.DISCONNECTED);
      engine.runLocalQuery.mockResolvedValue([
        { key: 'u1', value: { id: 'u1', name: 'LocalAda' } },
      ]);

      try {
        await client.queryOnce('users', {}, { allowLocal: true });
        throw new Error('expected queryOnce to throw QueryOnceLocalError');
      } catch (err) {
        expect(err).toBeInstanceOf(QueryOnceLocalError);
        const e = err as QueryOnceLocalError<{ name: string }>;
        expect(e.code).toBe('QUERY_ONCE_LOCAL_FALLBACK');
        expect(e.reason).toBe('offline');
        // The snapshot is reachable and unambiguously flagged non-settled by the error type.
        expect(Array.isArray(e.localData)).toBe(true);
        expect(e.localData[0]?._key).toBe('u1');
      }
    });

    test('allowLocal timeout → throws QueryOnceLocalError with reason "timeout"', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);
      engine.runLocalQuery.mockResolvedValue([{ key: 'u1', value: { id: 'u1' } }]);

      await expect(
        client.queryOnce('users', {}, { allowLocal: true, timeoutMs: 20 }),
      ).rejects.toMatchObject({
        code: 'QUERY_ONCE_LOCAL_FALLBACK',
        reason: 'timeout',
      });
    });

    test('settled server data is distinguishable from non-settled local: happy path resolves plainly', async () => {
      // Contract: a normal resolve is ALWAYS settled server data; only a thrown
      // QueryOnceLocalError is non-settled local. This is the distinguishing test.
      const { client, engine } = makeClient(SyncState.CONNECTED);
      const promise = client.queryOnce('users', {}, { allowLocal: true });
      await settleServer(engine, [{ key: 'u1', value: { id: 'u1', name: 'ServerAda' } }]);

      const results = await promise; // resolves, does not throw → settled server data
      expect(results[0]._key).toBe('u1');
      expect((results[0] as { name: string }).name).toBe('ServerAda');
    });
  });
});

describe('TopGunClient.queryOncePaged', () => {
  describe('AC3 — resolves to { items, cursor, hasMore }', () => {
    test('resolves with items, cursor, and hasMore from the server response', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);

      const promise = client.queryOncePaged('posts', { limit: 2 });

      await settleServerPaged(
        engine,
        [
          { key: 'p1', value: { title: 'Post 1' } },
          { key: 'p2', value: { title: 'Post 2' } },
        ],
        { nextCursor: 'cursor-abc', hasMore: true },
      );

      const result: QueryOncePagedResult<{ title: string }> = await promise;
      expect(result.items).toHaveLength(2);
      expect(result.items[0]._key).toBe('p1');
      expect(result.items[1]._key).toBe('p2');
      expect(result.cursor).toBe('cursor-abc');
      expect(result.hasMore).toBe(true);
    });

    test('hasMore is false and cursor is undefined when server has no further pages', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);

      const promise = client.queryOncePaged('posts', { limit: 10 });

      await settleServerPaged(engine, [{ key: 'p1', value: { title: 'Only Post' } }], {
        hasMore: false,
      });

      const result = await promise;
      expect(result.hasMore).toBe(false);
      expect(result.cursor).toBeUndefined();
      expect(result.items).toHaveLength(1);
    });

    test('auto-unsubscribes after resolving (no live subscription leak)', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);
      const promise = client.queryOncePaged('posts', {});
      await settleServerPaged(engine, [{ key: 'p1', value: {} }], { hasMore: false });
      await promise;

      expect(engine.unsubscribeFromQuery).toHaveBeenCalledTimes(1);
    });
  });

  describe('AC4 — offline policy matches queryOnce', () => {
    test('default offline → throws QueryOnceUnsettledError', async () => {
      const { client } = makeClient(SyncState.DISCONNECTED);

      await expect(client.queryOncePaged('posts', {})).rejects.toBeInstanceOf(
        QueryOnceUnsettledError,
      );
      await expect(client.queryOncePaged('posts', {})).rejects.toMatchObject({
        code: 'QUERY_ONCE_UNSETTLED',
        reason: 'offline',
      });
    });

    test('default timeout → throws QueryOnceUnsettledError (timeout)', async () => {
      const { client } = makeClient(SyncState.CONNECTED);

      await expect(client.queryOncePaged('posts', {}, { timeoutMs: 20 })).rejects.toMatchObject({
        code: 'QUERY_ONCE_UNSETTLED',
        reason: 'timeout',
      });
    });

    test('allowLocal offline → throws QueryOnceLocalError carrying local snapshot', async () => {
      const { client, engine } = makeClient(SyncState.DISCONNECTED);
      engine.runLocalQuery.mockResolvedValue([{ key: 'p1', value: { title: 'LocalPost' } }]);

      const promise = client.queryOncePaged('posts', {}, { allowLocal: true });
      await Promise.resolve();
      await Promise.resolve();

      await expect(promise).rejects.toBeInstanceOf(QueryOnceLocalError);
    });
  });
});

/**
 * A one-shot handle (queryOnce, queryOncePaged, loadMore's page handle) is sent
 * with `snapshotOnly: true` so the server registers no live subscription for it;
 * a live handle's payload is unchanged, and a live handle cannot carry
 * groupBy / aggregations at all.
 */
describe('snapshot-only one-shot queries', () => {
  /** A client whose engine double routes subscriptions through a real QueryManager. */
  function makeWiredClient(): {
    client: TopGunClient;
    engine: SyncEngineDouble;
    sent: jest.Mock;
    manager: QueryManager;
  } {
    const { client, engine } = makeClient(SyncState.CONNECTED);
    const sent = jest.fn(() => true);
    const manager = new QueryManager({
      storageAdapter: new MemoryStorageAdapter(),
      sendMessage: sent,
      isAuthenticated: () => true,
    });
    const capture = engine.subscribeToQuery.getMockImplementation()!;
    engine.subscribeToQuery.mockImplementation((handle: QueryHandle<unknown>) => {
      capture(handle);
      manager.subscribeToQuery(handle);
    });
    return { client, engine, sent, manager };
  }

  function querySubPayloads(sent: jest.Mock): Record<string, unknown>[] {
    return sent.mock.calls
      .map(([message]) => message as { type: string; payload: Record<string, unknown> })
      .filter((message) => message.type === 'QUERY_SUB')
      .map((message) => message.payload);
  }

  const grouped = {
    groupBy: ['category'],
    aggregations: [{ func: 'sum' as const, field: 'price' }],
  };

  test('grouped queryOnce sends snapshotOnly and resolves with the server rows', async () => {
    const { client, engine, sent } = makeWiredClient();
    const promise = client.queryOnce('orders', grouped);
    await settleServer(engine, [{ key: 'a', value: { __key: 'a', __count: 2, __sum_price: 35 } }]);
    const rows = await promise;
    expect(rows).toHaveLength(1);
    expect(rows[0]._key).toBe('a');
    const [payload] = querySubPayloads(sent);
    expect(payload.snapshotOnly).toBe(true);
    expect((payload.query as { groupBy: string[] }).groupBy).toEqual(['category']);
  });

  test('grouped queryOncePaged sends snapshotOnly and resolves with the server rows', async () => {
    const { client, engine, sent } = makeWiredClient();
    const promise = client.queryOncePaged('orders', grouped);
    await settleServerPaged(engine, [{ key: 'b', value: { __key: 'b', __count: 1 } }], {
      hasMore: false,
    });
    const result = await promise;
    expect(result.items.map((item) => item._key)).toEqual(['b']);
    expect(querySubPayloads(sent)[0].snapshotOnly).toBe(true);
  });

  test('plain queryOnce and queryOncePaged send snapshotOnly too', async () => {
    const { client, engine, sent } = makeWiredClient();
    const once = client.queryOnce('users', { where: { active: true } });
    await settleServer(engine, []);
    await once;
    const paged = client.queryOncePaged('users', { limit: 5 });
    await settleServerPaged(engine, [], { hasMore: false });
    await paged;
    const payloads = querySubPayloads(sent);
    expect(payloads).toHaveLength(2);
    expect(payloads.map((payload) => payload.snapshotOnly)).toEqual([true, true]);
  });

  test("loadMore's page handle sends snapshotOnly; the live handle does not", async () => {
    const { client, engine, sent } = makeWiredClient();
    const live = client.query('posts', { limit: 2, sort: { title: 'asc' } });
    const unsubscribe = live.subscribe(() => {});
    await Promise.resolve();
    live.updatePaginationInfo({ nextCursor: 'cursor-1', hasMore: true, cursorStatus: 'valid' });

    const more = live.loadMore();
    await settleServer(engine, [{ key: 'p3', value: { title: 'Post 3' } }]);
    await more;
    unsubscribe();

    const [livePayload, pagePayload] = querySubPayloads(sent);
    expect('snapshotOnly' in livePayload).toBe(false);
    expect(pagePayload.snapshotOnly).toBe(true);
    expect((pagePayload.query as { cursor: string }).cursor).toBe('cursor-1');
  });

  test('client.query with groupBy throws QueryError and sends nothing', () => {
    const { client, sent } = makeWiredClient();
    let caught: unknown;
    try {
      client.query('orders', { groupBy: ['category'] });
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(QueryError);
    expect((caught as QueryError).code).toBe('QUERY_LIVE_AGGREGATE_UNSUPPORTED');
    expect((caught as QueryError).mapName).toBe('orders');
    expect(querySubPayloads(sent)).toHaveLength(0);
  });

  test('a one-shot handle re-sent after reconnect keeps snapshotOnly', async () => {
    const { client, engine, sent, manager } = makeWiredClient();
    const promise = client.queryOnce('orders', grouped);
    await Promise.resolve();
    manager.resubscribeAll();
    await settleServer(engine, []);
    await promise;
    const payloads = querySubPayloads(sent);
    expect(payloads).toHaveLength(2);
    for (const payload of payloads) {
      expect(payload.snapshotOnly).toBe(true);
      expect((payload.query as QueryFilter).groupBy).toEqual(['category']);
    }
  });

});
