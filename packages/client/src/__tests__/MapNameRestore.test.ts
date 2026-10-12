import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { serialize, deserialize } from '@topgunbuild/core';
import type { WriteRejection } from '@topgunbuild/core';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import * as loggerModule from '../utils/logger';

/**
 * What a restart does with the pending operations it finds in local storage,
 * by map name.
 *
 * Two kinds of name can be sitting in a stored operation that a current client
 * would not create:
 *
 * - a name only the server refuses (here: one ending in the reserved `__backup`
 *   suffix). The restore keeps the operation and sends it; the server's
 *   refusal (code 400, permanent) is what retires it, and the application is
 *   told why, by name.
 * - a name the client itself cannot store under (here: one holding `:`). The
 *   restore does not send it, says which names it held back, and leaves the
 *   stored operation where it is.
 */

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  static CLOSED = 3;

  readyState: number = MockWebSocket.OPEN;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer | string }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: ((error: any) => void) | null = null;
  sentMessages: any[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    setTimeout(() => {
      if (this.onopen) this.onopen();
    }, 0);
  }
  send(data: Uint8Array | string) {
    if (data instanceof Uint8Array) this.sentMessages.push(deserialize(data));
    else this.sentMessages.push(JSON.parse(data));
  }
  close() {
    this.readyState = MockWebSocket.CLOSED;
    if (this.onclose) this.onclose({ code: 1000, reason: 'Normal closure' });
  }
  simulateMessage(message: any) {
    if (this.onmessage) {
      const data = serialize(message);
      this.onmessage({ data: new Uint8Array(data).buffer });
    }
  }
  static reset() {
    MockWebSocket.instances = [];
  }
  static getLastInstance(): MockWebSocket | undefined {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }
}
(global as any).WebSocket = MockWebSocket;

let uuidCounter = 0;
(global as any).crypto = { randomUUID: () => `test-uuid-${++uuidCounter}` };

const BACKFILL_DONE_KEY = '__sys__:ormapBackfillDone';

/**
 * In-memory adapter whose op log really shrinks: `deleteOp` and `markOpsSynced`
 * remove stored operations, so "the operation is gone from storage" and "the
 * operation is still in storage" are both things a test can observe.
 */
class DeletingAdapter implements IStorageAdapter {
  kv = new Map<string, any>();
  meta = new Map<string, any>();
  pending: OpLogEntry[] = [];

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  async get(key: string): Promise<any> {
    return this.kv.get(key);
  }
  async put(key: string, value: any): Promise<void> {
    this.kv.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.kv.delete(key);
  }
  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: any): Promise<void> {
    this.meta.set(key, value);
  }
  async batchPut(entries: Map<string, any>): Promise<void> {
    for (const [k, v] of entries) this.kv.set(k, v);
  }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.pending.reduce((max, op) => Math.max(max, Number(op.id)), 0) + 1;
    this.pending.push({ ...entry, id } as OpLogEntry);
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return [...this.pending];
  }
  async markOpsSynced(lastId: number): Promise<void> {
    this.pending = this.pending.filter((op) => Number(op.id) > lastId);
  }
  async deleteOp(id: number): Promise<void> {
    this.pending = this.pending.filter((op) => Number(op.id) !== id);
  }
  async commitWrite(): Promise<number> {
    return 1;
  }
  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kv.keys());
  }
  async getAllMetaKeys(): Promise<string[]> {
    return Array.from(this.meta.keys());
  }
}

function makeConfig(storageAdapter: IStorageAdapter): SyncEngineConfig {
  return {
    nodeId: 'test-node',
    connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
    storageAdapter,
    reconnectInterval: 1000,
    heartbeat: { enabled: false },
  };
}

const ts = () => ({ millis: Date.now(), counter: 0, nodeId: 'test' });

function storedPut(id: number, mapName: string, key: string): OpLogEntry {
  return {
    id,
    mapName,
    opType: 'PUT',
    key,
    record: { value: { v: id }, timestamp: ts() },
    synced: 0,
    timestamp: ts(),
  } as any;
}

// Held as a plain string so the comparison below type-checks against whatever
// set of causes the client's vocabulary has.
const INVALID_MAP_NAME_CAUSE: string = 'invalid_map_name';

describe('restoring pending operations by map name', () => {
  let syncEngine: SyncEngine | undefined;
  let storage: DeletingAdapter;
  let ws: MockWebSocket;
  let warnSpy: jest.SpyInstance;

  /** Boots an engine over `storage` and takes it through authentication, so the restored op log is flushed. */
  async function restoreAndConnect(): Promise<void> {
    syncEngine = new SyncEngine(makeConfig(storage));
    syncEngine.setAuthToken('test-token');
    await jest.runAllTimersAsync();

    ws = MockWebSocket.getLastInstance()!;
    ws.sentMessages = [];
    ws.simulateMessage({ type: 'AUTH_ACK' });
    await jest.runAllTimersAsync();
  }

  const opLogNames = (): string[] =>
    ((syncEngine as any).opLog as OpLogEntry[]).map((op) => op.mapName);
  const sentNames = (): string[] =>
    ws.sentMessages
      .filter((m) => m.type === 'OP_BATCH')
      .flatMap((m) => m.payload.ops.map((o: any) => o.mapName));

  beforeEach(() => {
    jest.useFakeTimers();
    MockWebSocket.reset();
    uuidCounter = 0;
    storage = new DeletingAdapter();
    // An already-migrated device, so the OR-Map backfill stays out of the way.
    storage.meta.set(BACKFILL_DONE_KEY, true);
    warnSpy = jest.spyOn(loggerModule.logger, 'warn');
  });

  afterEach(() => {
    if (syncEngine) {
      syncEngine.close();
      syncEngine = undefined;
    }
    warnSpy.mockRestore();
    jest.useRealTimers();
    jest.clearAllMocks();
  });

  describe('a stored operation whose name only the server refuses ("x__backup")', () => {
    const rejections: WriteRejection[] = [];

    /** Restores the op, lets it be sent, then delivers the server's permanent refusal for it. */
    async function restoreSendAndRefuse(): Promise<void> {
      rejections.length = 0;
      storage.pending = [storedPut(1, 'x__backup', 'k')];
      await restoreAndConnect();
      syncEngine!.onWriteRejected((rejection) => rejections.push(rejection));

      ws.simulateMessage({
        type: 'OP_REJECTED',
        payload: {
          opId: '1',
          reason: 'invalid map name: the "__backup" suffix is reserved',
          code: 400,
          permanent: true,
        },
      });
      // The durable delete and the op-log splice run on a promise chain.
      await jest.runAllTimersAsync();
      await (syncEngine as any).opRetirementChain;
    }

    it('is kept by the restore and sent in the next OP_BATCH', async () => {
      storage.pending = [storedPut(1, 'x__backup', 'k')];
      await restoreAndConnect();

      expect(opLogNames()).toEqual(['x__backup']);
      expect(sentNames()).toEqual(['x__backup']);
    });

    it('surfaces the refusal (code 400) as one write rejection with cause "invalid_map_name"', async () => {
      await restoreSendAndRefuse();

      expect(rejections).toHaveLength(1);
      expect(rejections[0].mapName).toBe('x__backup');
      const cause: string = rejections[0].cause;
      expect(cause).toBe(INVALID_MAP_NAME_CAUSE);
    });

    it('is retired by the refusal: gone from the op log and from storage', async () => {
      await restoreSendAndRefuse();

      expect(rejections).toHaveLength(1);
      expect(opLogNames()).toEqual([]);
      expect(await storage.getPendingOps()).toEqual([]);
    });
  });

  describe('a stored operation whose name the client cannot store under ("a:b")', () => {
    beforeEach(() => {
      storage.pending = [storedPut(1, 'a:b', 'k'), storedPut(2, 'users', 'u1')];
    });

    it('is not put into the op log and is not sent', async () => {
      await restoreAndConnect();

      expect(opLogNames()).toEqual(['users']);
      expect(sentNames()).toEqual(['users']);
    });

    it('is reported by the restore: the warning carries the count and the name', async () => {
      await restoreAndConnect();

      const shedWarnings = warnSpy.mock.calls.filter(
        ([payload, message]) =>
          typeof payload === 'object' &&
          payload !== null &&
          'count' in payload &&
          /map name/i.test(String(message)),
      );
      expect(shedWarnings).toHaveLength(1);
      expect(shedWarnings[0][0]).toEqual(expect.objectContaining({ count: 1, mapNames: ['a:b'] }));
    });

    it('is still in storage after the restore, before any acknowledgement arrives', async () => {
      await restoreAndConnect();

      // No OP_ACK has been delivered: the valid op is sent and unanswered.
      expect(ws.sentMessages.some((m) => m.type === 'OP_BATCH')).toBe(true);
      const stored = await storage.getPendingOps();
      expect(stored.map((op) => op.mapName)).toEqual(['a:b', 'users']);
    });
  });
});
