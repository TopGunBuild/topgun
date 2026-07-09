/**
 * Offline-durability regression suite (SPEC-321 — Client SDK depth-audit F3/F4/F5/F6).
 *
 * These are the permanent, INVERTED counterparts of the audit reproduction harness
 * (`client_sdk_repro.test.ts.txt`): each test asserts the FIXED behavior, and carries a
 * negative control proving the fix is load-bearing (the value really persists / the op is
 * really deleted / on commit failure neither layer is written).
 *
 * Findings keyed to DEPTH_CLIENT_SDK.md:
 *   F3 (TODO-496) — server-origin ORMap state is persisted (survives offline reload).
 *   F4 (TODO-499) — drop-oldest deletes the op from storage too (no resurrection).
 *   F5 (TODO-497) — oplog compacts on ack in BOTH memory and storage.
 *   F6 (TODO-498) — local write is atomic (commit-first ordering; no op-without-record).
 */
import { SyncEngine, SyncEngineConfig, OpLogEntry } from '../SyncEngine';
import { IStorageAdapter, StorageMutation } from '../IStorageAdapter';
import { BackpressureController } from '../sync/BackpressureController';
import { ORMapSyncHandler } from '../sync/ORMapSyncHandler';
import { DEFAULT_BACKPRESSURE_CONFIG } from '../BackpressureConfig';
import { ORMap, LWWMap, HLC, serialize, deserialize } from '@topgunbuild/core';
import type {
  IConnectionProvider,
  IConnection,
  ConnectionProviderEvent,
  ConnectionEventHandler,
} from '../types';

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- test shims a minimal global crypto for nodeId/uuid generation in jsdom-less node
(global as any).crypto = (global as any).crypto ?? { randomUUID: () => 'uuid-' + Math.random() };

// A fully controllable provider: we drive connected/message by hand.
class MockProvider implements IConnectionProvider {
  listeners = new Map<ConnectionProviderEvent, Set<ConnectionEventHandler>>();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- captured outbound frames are decoded msgpack of arbitrary message shape
  sent: any[] = [];
  connectedFlag = false;
  async connect(): Promise<void> {
    this.connectedFlag = true;
  }
  getConnection(): IConnection {
    return { send: () => {}, close: () => {}, readyState: 1 } as IConnection;
  }
  getAnyConnection(): IConnection {
    return this.getConnection();
  }
  isConnected(): boolean {
    return this.connectedFlag;
  }
  getConnectedNodes(): string[] {
    return this.connectedFlag ? ['default'] : [];
  }
  on(e: ConnectionProviderEvent, h: ConnectionEventHandler): void {
    if (!this.listeners.has(e)) this.listeners.set(e, new Set());
    this.listeners.get(e)!.add(h);
  }
  off(e: ConnectionProviderEvent, h: ConnectionEventHandler): void {
    this.listeners.get(e)?.delete(h);
  }
  send(data: ArrayBuffer | Uint8Array): void {
    this.sent.push(deserialize(data instanceof Uint8Array ? data : new Uint8Array(data)));
  }
  forceReconnect(): void {}
  async close(): Promise<void> {
    this.connectedFlag = false;
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- event args are heterogeneous per event type
  emit(e: ConnectionProviderEvent, ...args: any[]): void {
    this.listeners.get(e)?.forEach((h) => h(...args));
  }
}

/**
 * In-memory storage adapter with full commitWrite/deleteOp support, inspectable backing
 * maps, and a `failCommit` toggle that models an IndexedDB transaction abort (throws BEFORE
 * mutating any store, so neither the KV record nor the op is written — the atomicity contract).
 */
interface MemStore extends IStorageAdapter {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  __kv: Map<string, any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  __meta: Map<string, any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  __ops: any[];
  failCommit: boolean;
}
function memStorage(): MemStore {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const kv = new Map<string, any>();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const meta = new Map<string, any>();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const ops: any[] = [];
  let counter = 0;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const self: any = {
    failCommit: false,
    initialize: async () => {},
    close: async () => {},
    get: async (k: string) => kv.get(k),
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    put: async (k: string, v: any) => void kv.set(k, v),
    remove: async (k: string) => void kv.delete(k),
    getMeta: async (k: string) => meta.get(k),
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    setMeta: async (k: string, v: any) => void meta.set(k, v),
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    batchPut: async (e: Map<string, any>) => e.forEach((v, k) => kv.set(k, v)),
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    appendOpLog: async (e: any) => {
      const id = ++counter;
      ops.push({ ...e, id, synced: 0 });
      return id;
    },
    getPendingOps: async () => ops.filter((o) => o.synced === 0),
    // Delete-on-sync (matches IDBAdapter): remove acked rows, do not flag them.
    markOpsSynced: async (lastId: number) => {
      for (let i = ops.length - 1; i >= 0; i--) if (ops[i].id <= lastId) ops.splice(i, 1);
    },
    deleteOp: async (id: number) => {
      const i = ops.findIndex((o) => o.id === id);
      if (i >= 0) ops.splice(i, 1);
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    commitWrite: async (mutations: StorageMutation[], op: any) => {
      if (self.failCommit) throw new Error('injected commit failure');
      for (const m of mutations) {
        const target = m.store === 'meta' ? meta : kv;
        if (m.type === 'remove') target.delete(m.key);
        else target.set(m.key, m.value);
      }
      const id = ++counter;
      ops.push({ ...op, id, synced: 0 });
      return id;
    },
    getAllKeys: async () => Array.from(kv.keys()),
    __kv: kv,
    __meta: meta,
    __ops: ops,
  };
  return self as MemStore;
}

function makeEngine(storage: IStorageAdapter): { engine: SyncEngine; provider: MockProvider } {
  const provider = new MockProvider();
  const engine = new SyncEngine({
    nodeId: 'n1',
    connectionProvider: provider,
    storageAdapter: storage,
    heartbeat: { enabled: false },
  } as SyncEngineConfig);
  return { engine, provider };
}

describe('Offline durability (SPEC-321 — F3/F4/F5/F6)', () => {
  // ============================================================
  // F3 — server-origin ORMap state IS persisted (survives reload)
  // ============================================================
  describe('F3: server-origin ORMap persistence', () => {
    test('server OR_ADD persists the records array (symmetric with LWW PUT)', async () => {
      const storage = memStorage();
      const { engine, provider } = makeEngine(storage);

      const orMap = new ORMap<string, string>(engine.getHLC());
      engine.registerMap('tags', orMap);
      const lww = new LWWMap<string, string>(engine.getHLC());
      engine.registerMap('docs', lww);

      provider.emit(
        'message',
        'default',
        serialize({
          type: 'SERVER_EVENT',
          payload: {
            mapName: 'tags',
            eventType: 'OR_ADD',
            key: 'list1',
            orRecord: { value: 'work', tag: 't1', timestamp: engine.getHLC().now() },
          },
        }),
      );
      provider.emit(
        'message',
        'default',
        serialize({
          type: 'SERVER_EVENT',
          payload: {
            mapName: 'docs',
            eventType: 'PUT',
            key: 'd1',
            record: { value: 'hello', timestamp: engine.getHLC().now() },
          },
        }),
      );
      await new Promise((r) => setTimeout(r, 10));

      const kv = storage.__kv;
      // CONTROL: LWW server event persisted (unchanged).
      expect(kv.has('docs:d1')).toBe(true);
      // FIX (audit asserted false): ORMap server event IS persisted now.
      expect(kv.has('tags:list1')).toBe(true);
      // Negative control: the persisted value is the real records array, not a stub —
      // i.e. a reload would restore the tag.
      const stored = kv.get('tags:list1');
      expect(Array.isArray(stored)).toBe(true);
      expect(stored.some((r: { value: string }) => r.value === 'work')).toBe(true);
      // And memory still has it (the gap was durability-only).
      expect(orMap.get('list1')).toContain('work');
      engine.close();
    });

    test('server OR_REMOVE persists the tombstone set', async () => {
      const storage = memStorage();
      const { engine, provider } = makeEngine(storage);
      const orMap = new ORMap<string, string>(engine.getHLC());
      engine.registerMap('tags', orMap);

      provider.emit(
        'message',
        'default',
        serialize({
          type: 'SERVER_EVENT',
          payload: {
            mapName: 'tags',
            eventType: 'OR_ADD',
            key: 'list1',
            orRecord: { value: 'work', tag: 't1', timestamp: engine.getHLC().now() },
          },
        }),
      );
      provider.emit(
        'message',
        'default',
        serialize({
          type: 'SERVER_EVENT',
          payload: { mapName: 'tags', eventType: 'OR_REMOVE', key: 'list1', orTag: 't1' },
        }),
      );
      await new Promise((r) => setTimeout(r, 10));

      // Tombstone meta persisted under the canonical key.
      const tombstones = storage.__meta.get('__sys__:tags:tombstones');
      expect(Array.isArray(tombstones)).toBe(true);
      expect(tombstones).toContain('t1');
      engine.close();
    });

    test('ORMapSyncHandler diff merge persists each merged key + tombstones', async () => {
      const persistKey = jest.fn().mockResolvedValue(undefined);
      const persistTombstones = jest.fn().mockResolvedValue(undefined);
      const map = new ORMap<string, string>(new HLC('n1'));
      const handler = new ORMapSyncHandler({
        getMap: () => map,
        sendMessage: () => true,
        hlc: new HLC('n1'),
        onTimestampUpdate: async () => {},
        persistKey,
        persistTombstones,
        onCoveringEpochApplied: () => {},
        onFullResync: async () => {},
        getClaimedEpoch: () => 0,
      });

      await handler.handleORMapDiffResponse({
        mapName: 'tags',
        entries: [
          {
            key: 'list1',
            records: [{ value: 'work', tag: 't9', timestamp: new HLC('n1').now() }],
            tombstones: [],
          },
        ],
      });

      // FIX: the server-origin merge is persisted (audit: never persisted).
      expect(persistKey).toHaveBeenCalledWith('tags', 'list1');
      expect(persistTombstones).toHaveBeenCalledWith('tags');
      expect(map.get('list1')).toContain('work');
    });
  });

  // ============================================================
  // F5 — oplog compacts on ack (memory AND storage)
  // ============================================================
  describe('F5: oplog compaction on ack', () => {
    test('acked ops are spliced from memory and deleted from storage', async () => {
      const storage = memStorage();
      const { engine } = makeEngine(storage);
      const lww = new LWWMap<string, string>(engine.getHLC());
      engine.registerMap('m', lww);

      // Three durable local writes.
      for (let i = 0; i < 3; i++) {
        const rec = { value: `v${i}`, timestamp: engine.getHLC().now() };
        await engine.recordOperation(
          'm',
          'PUT',
          `k${i}`,
          { record: rec, timestamp: rec.timestamp },
          [{ store: 'kv', type: 'put', key: `m:k${i}`, value: rec }],
        );
      }

      // Negative control: BEFORE ack, all three ops are pending in both layers.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      expect((engine as any).opLog.length).toBe(3);
      expect(storage.__ops.length).toBe(3);

      // Server acks all ops up to the last id.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (engine as any).handleOpAck({ payload: { lastId: '3', achievedLevel: 1, results: [] } });
      await new Promise((r) => setTimeout(r, 5));

      // FIX: in-memory oplog spliced AND storage rows deleted (audit: neither compacts).
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      expect((engine as any).opLog.length).toBe(0);
      expect(storage.__ops.length).toBe(0);
      // The durable KV records remain — they are the source of truth.
      expect(storage.__kv.has('m:k0')).toBe(true);
      expect(storage.__kv.has('m:k2')).toBe(true);
      engine.close();
    });
  });

  // ============================================================
  // F6 — atomic local write (commit-first ordering)
  // ============================================================
  describe('F6: atomic local write', () => {
    test('successful write persists BOTH the record and its op', async () => {
      const storage = memStorage();
      const { engine } = makeEngine(storage);
      const rec = { value: 'hi', timestamp: engine.getHLC().now() };
      await engine.recordOperation('m', 'PUT', 'k1', { record: rec, timestamp: rec.timestamp }, [
        { store: 'kv', type: 'put', key: 'm:k1', value: rec },
      ]);

      expect(storage.__kv.has('m:k1')).toBe(true);
      expect(storage.__ops.length).toBe(1);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      expect((engine as any).opLog.length).toBe(1);
      engine.close();
    });

    test('mid-commit failure leaves NEITHER the record nor the op (no op-without-record)', async () => {
      const storage = memStorage();
      const { engine } = makeEngine(storage);
      storage.failCommit = true;

      const rec = { value: 'hi', timestamp: engine.getHLC().now() };
      await expect(
        engine.recordOperation('m', 'PUT', 'k1', { record: rec, timestamp: rec.timestamp }, [
          { store: 'kv', type: 'put', key: 'm:k1', value: rec },
        ]),
      ).rejects.toThrow('injected commit failure');

      // FIX: atomic commit → neither store written.
      expect(storage.__kv.has('m:k1')).toBe(false);
      expect(storage.__ops.length).toBe(0);
      // Commit-first ordering: the in-memory oplog was NOT pushed on failure (no op-without-record).
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      expect((engine as any).opLog.length).toBe(0);
      engine.close();
    });
  });

  // ============================================================
  // F4 — durable drop-oldest (memory + storage agree)
  // ============================================================
  describe('F4: durable drop-oldest', () => {
    test('controller deletes the dropped op via onOpDropped (memory + storage)', async () => {
      const storage = memStorage();
      // Seed storage with two ops mirroring the in-memory opLog (IStorageAdapter op shape).
      await storage.appendOpLog({ mapName: 'm', op: 'PUT', key: '1', synced: 0 });
      await storage.appendOpLog({ mapName: 'm', op: 'PUT', key: '2', synced: 0 });

      const opLog: OpLogEntry[] = [];
      const dropped: string[] = [];
      const ctrl = new BackpressureController({
        config: { ...DEFAULT_BACKPRESSURE_CONFIG, strategy: 'drop-oldest', maxPendingOps: 2 },
        opLog,
        onOpDropped: (opId: string) => {
          dropped.push(opId);
          const n = parseInt(opId, 10);
          if (!isNaN(n)) void storage.deleteOp(n);
        },
      });
      const mk = (id: string): OpLogEntry => ({
        id,
        mapName: 'm',
        opType: 'PUT',
        key: id,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        timestamp: { millis: 1, counter: 0, nodeId: 'n' } as any,
        synced: false,
      });
      opLog.push(mk('1'), mk('2'));

      await ctrl.checkBackpressure(); // at capacity → drop oldest ('1')

      // Memory: op '1' gone.
      expect(opLog.map((o) => o.id)).toEqual(['2']);
      // FIX: onOpDropped fired and storage op 1 deleted (audit: controller had no storage path).
      expect(dropped).toEqual(['1']);
      await new Promise((r) => setTimeout(r, 5));
      expect(storage.__ops.map((o) => o.id)).toEqual([2]);
    });
  });
});
