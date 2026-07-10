import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { serialize, deserialize } from '@topgunbuild/core';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import * as loggerModule from '../utils/logger';
import { assertValidMapName } from '../utils/mapName';

// --- Mock WebSocket (mirrors SyncEngine.test.ts) ---
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

// Stateful in-memory adapter so backfill markers written via setMeta are visible
// to the subsequent getAllMetaKeys held-set enumeration (the jest.fn mock in
// SyncEngine.test.ts intentionally does not round-trip meta writes).
class MemAdapter implements IStorageAdapter {
  kv = new Map<string, any>();
  meta = new Map<string, any>();
  pending: OpLogEntry[] = [];
  throwOnGet = new Set<string>();

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  async get<V>(key: string): Promise<any> {
    if (this.throwOnGet.has(key)) throw new Error(`injected read failure for ${key}`);
    return this.kv.get(key) as V | undefined;
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
    const id = this.pending.length + 1;
    this.pending.push({ ...entry, id } as OpLogEntry);
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.pending;
  }
  async markOpsSynced(): Promise<void> {}
  async deleteOp(): Promise<void> {}
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
const orRecord = (value: string, tag: string) => [{ value, tag, timestamp: ts() }];

describe('SPEC-343 SyncEngine map-name folds', () => {
  let syncEngine: SyncEngine | undefined;

  beforeEach(() => {
    jest.useFakeTimers();
    MockWebSocket.reset();
    uuidCounter = 0;
  });

  afterEach(() => {
    if (syncEngine) {
      syncEngine.close();
      syncEngine = undefined;
    }
    jest.useRealTimers();
    jest.clearAllMocks();
  });

  // AC3b — no phantom held-name from an ordinary composite KEY.
  describe('AC3b — composite KEY never produces a phantom held-name', () => {
    it('key "post:123" under map "tags" yields held name "tags", NOT "tags:post"', async () => {
      const storage = new MemAdapter();
      // Real OR-Map "tags" with composite key "post:123" → storage key "tags:post:123".
      storage.kv.set('tags:post:123', orRecord('x', 't1'));
      // Backfill NOT yet done → the first-colon backfill runs during the snapshot.

      syncEngine = new SyncEngine(makeConfig(storage));
      await jest.runAllTimersAsync();
      await (syncEngine as any).startMerkleSync();

      const held: Set<string> = (syncEngine as any).heldOrMapNames;
      expect(held.has('tags')).toBe(true);
      expect(held.has('tags:post')).toBe(false);

      // The first-colon backfill stamps only "tags" — never a phantom "tags:post".
      expect(storage.meta.has('__sys__:tags:ormap')).toBe(true);
      expect(storage.meta.has('__sys__:tags:post:ormap')).toBe(false);

      // No phantom map is instantiated (so no sync-init/push can ever run for it).
      const maps: Map<string, unknown> = (syncEngine as any).maps;
      expect(maps.has('tags')).toBe(true);
      expect(maps.has('tags:post')).toBe(false);
    });
  });

  // AC4 — unreachability of an add-only, unmarked legacy colon-named map "a:b".
  describe('AC4 — legacy colon-named "a:b" is unreachable (unheld, uninstantiated)', () => {
    it('held-set surfaces "a" but not "a:b"; no marker/instance/ACK path under "a:b"', async () => {
      const storage = new MemAdapter();
      storage.kv.set('a:k', orRecord('own', 't-a'));
      storage.kv.set('a:b:k', orRecord('sibling', 't-ab')); // the legacy colon-named store

      syncEngine = new SyncEngine(makeConfig(storage));
      await jest.runAllTimersAsync();
      await (syncEngine as any).startMerkleSync();

      const held: Set<string> = (syncEngine as any).heldOrMapNames;
      expect(held.has('a')).toBe(true);
      // First-colon backfill attributes "a:b:k" to "a" → "a:b" is never surfaced.
      expect(held.has('a:b')).toBe(false);

      // No "a:b" existence marker is ever stamped.
      expect(storage.meta.has('__sys__:a:ormap')).toBe(true);
      expect(storage.meta.has('__sys__:a:b:ormap')).toBe(false);

      // "a:b" is never instantiated → no CLIENT_APPLY_ACK path runs under it.
      const maps: Map<string, unknown> = (syncEngine as any).maps;
      expect(maps.has('a:b')).toBe(false);

      // And it can never be opened: getORMap('a:b') is forbidden at the boundary.
      expect(() => assertValidMapName('a:b')).toThrow(/":"/);
    });
  });

  // AC5 — loadOpLog name-filter (RED without the fix).
  describe('AC5 — a persisted op under a forbidden name is dropped at loadOpLog', () => {
    it('drops mapName "a:b" (warn+count) and never flushes it in OP_BATCH', async () => {
      const storage = new MemAdapter();
      // Already-migrated device so the held-set enumeration stays inert for this test.
      storage.meta.set(BACKFILL_DONE_KEY, true);
      storage.pending = [
        { id: 1, mapName: 'a:b', opType: 'PUT', key: 'k', synced: 0, timestamp: ts() } as any,
        { id: 2, mapName: 'users', opType: 'PUT', key: 'u1', synced: 0, timestamp: ts() } as any,
      ];

      const warnSpy = jest.spyOn(loggerModule.logger, 'warn');

      syncEngine = new SyncEngine(makeConfig(storage));
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.sentMessages = [];
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      const opBatch = ws.sentMessages.find((m) => m.type === 'OP_BATCH');
      expect(opBatch).toBeDefined();
      const mapNames = opBatch.payload.ops.map((o: any) => o.mapName);
      // The forbidden-name op was shed at load — only the valid op flushes.
      // (Pre-fix loadOpLog had NO name-filter, so BOTH ops would flush here and
      // this assertion would fail — the RED-without-fix property.)
      expect(mapNames).toEqual(['users']);
      expect(mapNames).not.toContain('a:b');

      expect(warnSpy).toHaveBeenCalledWith(
        expect.objectContaining({ count: 1 }),
        expect.stringContaining('invalid (non-injective) map name'),
      );
      warnSpy.mockRestore();
    });
  });

  // AC5b — longest-held-name restore-guard inside instantiateAndRestoreOrMap.
  describe('AC5b — instantiateAndRestoreOrMap skips a key owned by a LONGER held name', () => {
    it('restoring "a" with both "a" and "a:b" held does NOT absorb "a:b:k"', async () => {
      const storage = new MemAdapter();
      storage.kv.set('a:k1', orRecord('own', 't-own'));
      storage.kv.set('a:b:k', orRecord('sibling', 't-sib'));

      syncEngine = new SyncEngine(makeConfig(storage));
      await jest.runAllTimersAsync();

      // Reset the registry so instantiateAndRestoreOrMap runs a fresh restore (the
      // boot-time connection flow may already have instantiated "a" under a held-set
      // that did not yet include the longer name).
      (syncEngine as any).maps.clear();
      // Drive the shared discriminator: both names held for this connection.
      (syncEngine as any).heldOrMapNames = new Set(['a', 'a:b']);

      const mapA = await (syncEngine as any).instantiateAndRestoreOrMap('a');

      expect(mapA.get('k1')).toEqual(['own']);
      // "b:k" belongs to the LONGER held name "a:b" → skipped, not merged into "a".
      expect(mapA.get('b:k')).toEqual([]);
    });
  });

  // AC6 — backfill fail-closed preserved.
  describe('AC6 — a storage error mid-backfill fail-closes the connection', () => {
    it('leaves the done-flag unset and sets heldSetIncomplete (ACKs suppressed)', async () => {
      const storage = new MemAdapter();
      storage.kv.set('a:k', orRecord('v', 't'));
      storage.throwOnGet.add('a:k'); // read failure mid-scan
      // Backfill-done deliberately UNSET so the scan actually runs.

      syncEngine = new SyncEngine(makeConfig(storage));
      await jest.runAllTimersAsync();
      await (syncEngine as any).startMerkleSync();

      expect((syncEngine as any).heldSetIncomplete).toBe(true);
      // The done-flag is set ONLY on a fully-successful scan → it stays unset here.
      expect(storage.meta.has(BACKFILL_DONE_KEY)).toBe(false);
    });
  });
});
