import { TopGunClient } from '../TopGunClient';
import { QueryHandle } from '../QueryHandle';
import { SyncState } from '../SyncState';
import {
  QueryOnceUnsettledError,
  QueryOnceLocalError,
} from '../errors/QueryOnceError';
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
  }
  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kv.keys());
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

describe('TopGunClient.queryOnce', () => {
  describe('AC1 — returns authoritative server data not local []', () => {
    test('resolves with a server-only record (not [])', async () => {
      const { client, engine } = makeClient(SyncState.CONNECTED);
      // Local store has nothing for this query — record exists ONLY on the server.
      engine.runLocalQuery.mockResolvedValue([]);

      const promise = client.queryOnce('users', {});

      await settleServer(engine, [
        { key: 'u1', value: { id: 'u1', name: 'Ada' } },
      ]);

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

      await expect(client.queryOnce('users', {})).rejects.toBeInstanceOf(
        QueryOnceUnsettledError,
      );
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

      await expect(client.queryOnce('users', {})).rejects.toBeInstanceOf(
        QueryOnceUnsettledError,
      );
    });

    test('default timeout → REJECTS with QueryOnceUnsettledError (timeout)', async () => {
      const { client } = makeClient(SyncState.CONNECTED);
      // Online but server never settles within the window.
      await expect(
        client.queryOnce('users', {}, { timeoutMs: 20 }),
      ).rejects.toMatchObject({
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
      engine.runLocalQuery.mockResolvedValue([
        { key: 'u1', value: { id: 'u1' } },
      ]);

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
