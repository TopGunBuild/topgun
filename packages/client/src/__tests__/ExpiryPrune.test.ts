import { serialize, deserialize, HLC, LWWMap, MerkleTree, ORMap } from '@topgunbuild/core';
import type { LWWRecord, ORMapRecord } from '@topgunbuild/core';
import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import type { IStorageAdapter } from '../IStorageAdapter';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { NullConnectionProvider } from '../connection/NullConnectionProvider';
import { ORMapSyncHandler } from '../sync/ORMapSyncHandler';
import { logger } from '../utils/logger';

/**
 * Client-side expiry of LWW records (TG-TTL-002).
 *
 * The server's sweep deletes an expired LWW key and then forgets its leaf, and
 * LWW sync only pulls, so the client has to drop the record itself, from memory,
 * from its Merkle tree and from storage, or its root would never match the
 * server's again. OR maps are left alone: the server's expiry tombstone reaches
 * them through the ordinary OR sync.
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
  onerror: ((error: unknown) => void) | null = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- decoded frames have no fixed shape at the test boundary
  sentMessages: any[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    setTimeout(() => this.onopen?.(), 0);
  }

  send(data: Uint8Array | string) {
    if (this.readyState !== MockWebSocket.OPEN) throw new Error('WebSocket is not open');
    this.sentMessages.push(data instanceof Uint8Array ? deserialize(data) : JSON.parse(data));
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code: 1000, reason: 'Normal closure' });
  }

  simulateMessage(message: unknown) {
    this.onmessage?.({ data: new Uint8Array(serialize(message)).buffer });
  }

  static last(): MockWebSocket {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the engine reads WebSocket from the global scope
(global as any).WebSocket = MockWebSocket;

/** A storage adapter backed by a Map, so a "reload" is a read of what is left in it. */
function createStorage(): jest.Mocked<IStorageAdapter> & { kv: Map<string, unknown> } {
  const kv = new Map<string, unknown>();
  return {
    kv,
    initialize: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    get: jest.fn((key: string) => Promise.resolve(kv.get(key))),
    put: jest.fn((key: string, value: unknown) => {
      kv.set(key, value);
      return Promise.resolve();
    }),
    remove: jest.fn((key: string) => {
      kv.delete(key);
      return Promise.resolve();
    }),
    getMeta: jest
      .fn()
      .mockImplementation((key: string) =>
        Promise.resolve(key === '__sys__:ormapBackfillDone' ? true : undefined),
      ),
    setMeta: jest.fn().mockResolvedValue(undefined),
    batchPut: jest.fn().mockResolvedValue(undefined),
    appendOpLog: jest.fn().mockResolvedValue(1),
    getPendingOps: jest.fn().mockResolvedValue([]),
    markOpsSynced: jest.fn().mockResolvedValue(undefined),
    deleteOp: jest.fn().mockResolvedValue(undefined),
    commitWrite: jest.fn().mockResolvedValue(1),
    getAllKeys: jest.fn(() => Promise.resolve(Array.from(kv.keys()))),
    getAllMetaKeys: jest.fn().mockResolvedValue([]),
  } as jest.Mocked<IStorageAdapter> & { kv: Map<string, unknown> };
}

const T0 = 1_800_000_000_000;
const MAP = 'users';

const serverRecord = (millis: number, ttlMs?: number, value = 'v'): LWWRecord<string> => ({
  value,
  timestamp: { millis, counter: 0, nodeId: 'server' },
  ...(ttlMs !== undefined ? { ttlMs } : {}),
});

/** Puts a server record into the map and into storage, as a pulled record would be. */
function seed(
  map: LWWMap<string, string>,
  storage: { kv: Map<string, unknown> },
  key: string,
  record: LWWRecord<string>,
) {
  map.merge(key, record);
  storage.kv.set(`${MAP}:${key}`, record);
}

const emptyRoot = () => new MerkleTree().getRootHash();

describe('client-side LWW expiry prune', () => {
  let engine: SyncEngine | undefined;
  let storage: ReturnType<typeof createStorage>;

  const build = (online: boolean): SyncEngine => {
    const config: SyncEngineConfig = {
      nodeId: 'client',
      connectionProvider: online
        ? new SingleServerProvider({ url: 'ws://localhost:8080' })
        : new NullConnectionProvider(),
      storageAdapter: storage,
      heartbeat: { enabled: false },
    };
    engine = new SyncEngine(config);
    return engine;
  };

  /**
   * Lets the token-less handshake finish, so the connection's own SYNC_INIT
   * frames are sent before the test's maps exist.
   */
  const connected = async (): Promise<MockWebSocket> => {
    await jest.advanceTimersByTimeAsync(600);
    return MockWebSocket.last();
  };

  const openMap = (e: SyncEngine): LWWMap<string, string> => {
    const map = new LWWMap<string, string>(e.getHLC());
    e.registerMap(MAP, map);
    return map;
  };

  /** Messages the client sent other than its handshake. */
  const sentTypes = (ws: MockWebSocket): string[] =>
    ws.sentMessages.map((m) => m.type).filter((t) => t !== 'DEVICE_HELLO' && t !== 'AUTH');

  beforeEach(() => {
    jest.useFakeTimers();
    jest.setSystemTime(T0);
    MockWebSocket.instances = [];
    storage = createStorage();
  });

  afterEach(() => {
    engine?.close();
    engine = undefined;
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  it('lww_prune_expired_sends_no_op: the timer drops the record, writes no op and sends nothing', async () => {
    const e = build(true);
    const ws = await connected();
    const map = openMap(e);
    seed(map, storage, 'gone', serverRecord(T0 - 5_000, 1_000));
    seed(map, storage, 'kept', serverRecord(T0 - 5_000));
    ws.sentMessages = [];

    await jest.advanceTimersByTimeAsync(1_000);

    expect(map.getRecord('gone')).toBeUndefined();
    expect(map.getRecord('kept')).toBeDefined();
    expect(storage.remove).toHaveBeenCalledWith(`${MAP}:gone`);
    expect(e.getPendingOpsCount()).toBe(0);
    expect(storage.commitWrite).not.toHaveBeenCalled();
    expect(storage.appendOpLog).not.toHaveBeenCalled();
    expect(sentTypes(ws)).toEqual([]);
  });

  it('prune_expired_survives_reload: the record is gone from storage', async () => {
    const e = build(false);
    const map = openMap(e);
    seed(map, storage, 'gone', serverRecord(T0 - 5_000, 1_000));
    seed(map, storage, 'kept', serverRecord(T0 - 5_000, 60_000));

    await jest.advanceTimersByTimeAsync(1_000);

    expect(Array.from(storage.kv.keys())).toEqual([`${MAP}:kept`]);
  });

  it('a failed storage removal is logged and the run goes on with the next key', async () => {
    const warn = jest.spyOn(logger, 'warn');
    const e = build(false);
    const map = openMap(e);
    seed(map, storage, 'a', serverRecord(T0 - 5_000, 1_000));
    seed(map, storage, 'b', serverRecord(T0 - 5_000, 1_000));
    storage.remove.mockImplementationOnce(() => Promise.reject(new Error('disk')));

    await jest.advanceTimersByTimeAsync(1_000);

    expect(map.getRecord('a')).toBeUndefined();
    expect(map.getRecord('b')).toBeUndefined();
    expect(storage.remove).toHaveBeenCalledTimes(2);
    expect(warn).toHaveBeenCalledWith(
      expect.objectContaining({ mapName: MAP }),
      expect.stringContaining('expired record'),
    );
  });

  it('lww_prune_expired_skips_key_with_pending_put and prunes it once the PUT is acknowledged', async () => {
    const e = build(true);
    const ws = await connected();
    const map = openMap(e);
    const record = map.set('k', 'mine', 1_000);
    const opId = await e.recordOperation(MAP, 'PUT', 'k', {
      record,
      timestamp: record.timestamp,
    });

    await jest.advanceTimersByTimeAsync(3_000);
    expect(map.get('k')).toBeUndefined();
    expect(map.getRecord('k')).toBeDefined();

    ws.simulateMessage({
      type: 'OP_ACK',
      payload: { lastId: opId, results: [{ opId, success: true }] },
    });
    await jest.advanceTimersByTimeAsync(1_000);

    expect(e.getPendingOpsCount()).toBe(0);
    expect(map.getRecord('k')).toBeUndefined();
  });

  it('prune_timer_runs_while_offline and stops on close()', async () => {
    const e = build(false);
    const map = openMap(e);
    seed(map, storage, 'a', serverRecord(T0 - 5_000, 1_000));

    await jest.advanceTimersByTimeAsync(999);
    expect(map.getRecord('a')).toBeDefined();
    await jest.advanceTimersByTimeAsync(1);
    expect(map.getRecord('a')).toBeUndefined();

    e.close();
    seed(map, storage, 'b', serverRecord(T0 - 5_000, 1_000));
    const late = new LWWMap<string, string>(e.getHLC());
    e.registerMap('late', late);
    late.merge('c', serverRecord(T0 - 5_000, 1_000));
    await jest.advanceTimersByTimeAsync(5_000);
    expect(map.getRecord('b')).toBeDefined();
    expect(late.getRecord('c')).toBeDefined();
  });

  it('starts no timer until an LWW map is registered', async () => {
    const e = build(false);
    expect(jest.getTimerCount()).toBe(0);
    e.registerMap('tags', new ORMap<string, string>(e.getHLC()));
    await jest.advanceTimersByTimeAsync(0);
    expect(jest.getTimerCount()).toBe(0);
    openMap(e);
    expect(jest.getTimerCount()).toBe(1);
  });

  describe('pre-root prune', () => {
    /** Holds every storage removal until `release()` is called. */
    const holdRemovals = () => {
      let release!: () => void;
      const gate = new Promise<void>((resolve) => (release = resolve));
      storage.remove.mockImplementation(async (key: string) => {
        await gate;
        storage.kv.delete(key);
      });
      return () => release();
    };

    it('pre_root_prune_persists_before_root_read', async () => {
      const e = build(true);
      const ws = await connected();
      const map = openMap(e);
      seed(map, storage, 'gone', serverRecord(T0 - 5_000, 1_000));
      const rootRead = jest.spyOn(map.getMerkleTree(), 'getRootHash');
      const release = holdRemovals();
      ws.sentMessages = [];

      ws.simulateMessage({ type: 'SYNC_RESP_ROOT', payload: { mapName: MAP, rootHash: 12345 } });
      await jest.advanceTimersByTimeAsync(0);
      expect(rootRead).not.toHaveBeenCalled();
      expect(sentTypes(ws)).toEqual([]);

      release();
      await jest.advanceTimersByTimeAsync(0);
      expect(storage.kv.has(`${MAP}:gone`)).toBe(false);
      expect(rootRead).toHaveBeenCalled();
      expect(sentTypes(ws)).toEqual(['MERKLE_REQ_BUCKET']);
    });

    it('waits for a timer run whose removals are still pending', async () => {
      const e = build(true);
      const ws = await connected();
      const map = openMap(e);
      seed(map, storage, 'gone', serverRecord(T0 - 5_000, 1_000));
      const release = holdRemovals();
      await jest.advanceTimersByTimeAsync(1_000);
      expect(map.getRecord('gone')).toBeUndefined();
      expect(storage.kv.has(`${MAP}:gone`)).toBe(true);
      const rootRead = jest.spyOn(map.getMerkleTree(), 'getRootHash');
      ws.sentMessages = [];

      ws.simulateMessage({ type: 'SYNC_RESP_ROOT', payload: { mapName: MAP, rootHash: 12345 } });
      await jest.advanceTimersByTimeAsync(0);
      expect(rootRead).not.toHaveBeenCalled();
      expect(sentTypes(ws)).toEqual([]);

      release();
      await jest.advanceTimersByTimeAsync(0);
      expect(rootRead).toHaveBeenCalled();
      expect(sentTypes(ws)).toEqual(['MERKLE_REQ_BUCKET']);
    });

    it('reads the root of the pruned map: a server that swept the key is in sync', async () => {
      const e = build(true);
      const ws = await connected();
      const map = openMap(e);
      seed(map, storage, 'gone', serverRecord(T0 - 5_000, 1_000));
      ws.sentMessages = [];

      ws.simulateMessage({
        type: 'SYNC_RESP_ROOT',
        payload: { mapName: MAP, rootHash: emptyRoot() },
      });
      await jest.advanceTimersByTimeAsync(0);

      expect(map.getMerkleTree().getRootHash()).toBe(emptyRoot());
      expect(sentTypes(ws)).toEqual([]);
    });

    it('still walks when a storage removal fails', async () => {
      const e = build(true);
      const ws = await connected();
      const map = openMap(e);
      seed(map, storage, 'gone', serverRecord(T0 - 5_000, 1_000));
      storage.remove.mockImplementation(() => Promise.reject(new Error('disk')));
      ws.sentMessages = [];

      ws.simulateMessage({ type: 'SYNC_RESP_ROOT', payload: { mapName: MAP, rootHash: 12345 } });
      await jest.advanceTimersByTimeAsync(0);

      expect(sentTypes(ws)).toEqual(['MERKLE_REQ_BUCKET']);
    });
  });

  describe('interim states of TG-TTL-002', () => {
    /**
     * Runs one root/leaf exchange in which the server still serves `key` with
     * `served`, and returns what the client sent.
     */
    const walk = async (ws: MockWebSocket, key: string, served: LWWRecord<string>) => {
      ws.sentMessages = [];
      ws.simulateMessage({ type: 'SYNC_RESP_ROOT', payload: { mapName: MAP, rootHash: 12345 } });
      await jest.advanceTimersByTimeAsync(0);
      ws.simulateMessage({
        type: 'SYNC_RESP_LEAF',
        payload: { mapName: MAP, path: '', records: [{ key, record: served }] },
      });
      await jest.advanceTimersByTimeAsync(0);
      return sentTypes(ws);
    };

    it('(a) client pruned, server not swept: shows the server record until its own expiry', async () => {
      const e = build(true);
      const ws = await connected();
      const map = openMap(e);
      seed(map, storage, 'k', serverRecord(T0 - 5_000, 1_000, 'old'));
      await jest.advanceTimersByTimeAsync(1_000);
      expect(map.getRecord('k')).toBeUndefined();

      // The server's copy carries a later receipt stamp and is not expired.
      const served = serverRecord(T0 - 500, 10_000, 'server');
      expect(await walk(ws, 'k', served)).toEqual(['MERKLE_REQ_BUCKET']);
      expect(map.get('k')).toBe('server');

      // Every tick up to the server-anchored expiry instant keeps it.
      await jest.advanceTimersByTimeAsync(served.timestamp.millis + 10_000 - Date.now());
      expect(map.get('k')).toBe('server');
      await jest.advanceTimersByTimeAsync(1_000);
      expect(map.getRecord('k')).toBeUndefined();

      expect(e.getPendingOpsCount()).toBe(0);
      expect(storage.commitWrite).not.toHaveBeenCalled();
      expect(sentTypes(ws)).toEqual(['MERKLE_REQ_BUCKET']);
    });

    it('(b) an offline write acknowledged later: the server-stamped record lives to its own expiry, then roots match', async () => {
      const e = build(true);
      const ws = await connected();
      const map = openMap(e);
      const local = map.set('k', 'mine', 1_000);
      const opId = await e.recordOperation(MAP, 'PUT', 'k', {
        record: local,
        timestamp: local.timestamp,
      });
      ws.simulateMessage({
        type: 'OP_ACK',
        payload: { lastId: opId, results: [{ opId, success: true }] },
      });
      await jest.advanceTimersByTimeAsync(2_000);
      expect(map.getRecord('k')).toBeUndefined();

      // The server stamped the write on receipt, later than the local stamp.
      const served = serverRecord(local.timestamp.millis + 1_500, 1_000, 'mine');
      expect(await walk(ws, 'k', served)).toEqual(['MERKLE_REQ_BUCKET']);
      expect(map.get('k')).toBe('mine');

      ws.sentMessages = [];
      await jest.advanceTimersByTimeAsync(served.timestamp.millis + 1_000 - Date.now());
      expect(map.get('k')).toBe('mine');
      await jest.advanceTimersByTimeAsync(1_000);
      expect(map.getRecord('k')).toBeUndefined();
      expect(map.getMerkleTree().getRootHash()).toBe(emptyRoot());
      expect(sentTypes(ws)).toEqual([]);
      expect(e.getPendingOpsCount()).toBe(0);
    });
  });
});

describe('OR maps get no client-side expiry write', () => {
  afterEach(() => {
    jest.useRealTimers();
  });

  it('keeps an expired tag (hidden), tombstones nothing, and pushes it only as a live record', async () => {
    jest.useFakeTimers();
    jest.setSystemTime(T0);
    const storage = createStorage();
    const engine = new SyncEngine({
      nodeId: 'client',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: storage,
      heartbeat: { enabled: false },
    });
    const map = new ORMap<string, string>(engine.getHLC());
    engine.registerMap('tags', map);
    // An LWW map too, so the prune timer runs.
    engine.registerMap(MAP, new LWWMap<string, string>(engine.getHLC()));

    const expired: ORMapRecord<string> = {
      value: 'x',
      tag: `${T0 - 5_000}:0:server`,
      timestamp: { millis: T0 - 5_000, counter: 0, nodeId: 'server' },
      ttlMs: 1_000,
    };
    map.apply('k', expired);
    await jest.advanceTimersByTimeAsync(3_000);

    expect(map.get('k')).toEqual([]);
    expect(map.getRecordsMap('k')?.get(expired.tag)).toEqual(expired);
    expect(map.getTombstones()).toEqual([]);
    expect(storage.commitWrite).not.toHaveBeenCalled();
    engine.close();
    jest.useRealTimers();

    // A leaf response that serves the tag live makes the handler push the key back.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- captured frames have no fixed shape at the test boundary
    const sent: any[] = [];
    const handler = new ORMapSyncHandler({
      getMap: () => map,
      sendMessage: (msg) => {
        sent.push(msg);
        return true;
      },
      hlc: new HLC('client'),
      onTimestampUpdate: async () => {},
      persistKey: async () => {},
      persistTombstones: async () => {},
      getPendingRemoveTagsByKey: () => new Map(),
      persistKeyTombstones: async () => {},
      onCoveringEpochApplied: () => {},
      onFullResync: async () => {},
      getClaimedEpoch: () => 0,
    });
    await handler.handleORMapSyncRespLeaf({
      mapName: 'tags',
      entries: [{ key: 'k', records: [expired], tombstones: [] }],
    });

    const pushes = sent.filter((m) => m.type === 'ORMAP_PUSH_DIFF');
    expect(pushes).toHaveLength(1);
    for (const entry of pushes[0].payload.entries) {
      expect(entry.tombstones).not.toContain(expired.tag);
    }
    expect(pushes[0].payload.entries).toEqual([
      {
        key: 'k',
        records: [
          expect.objectContaining({ tag: expired.tag, timestamp: expired.timestamp, ttlMs: 1_000 }),
        ],
        tombstones: [],
      },
    ]);
    expect(map.getTombstones()).toEqual([]);
  });
});
