import { HLC, LWWMap, MerkleTree } from '@topgunbuild/core';
import type { LWWRecord, Timestamp } from '@topgunbuild/core';
import { SyncEngine } from '../SyncEngine';
import type { IStorageAdapter, OpLogEntry, StorageMutation } from '../IStorageAdapter';
import { NullConnectionProvider } from '../connection/NullConnectionProvider';
import { TopGunClient } from '../TopGunClient';
import { logger } from '../utils/logger';

/**
 * An LWW REMOVE the server broadcasts carries no record (TG-LWW-001). The
 * client forgets the key — record, Merkle leaf and storage entry — unless it
 * holds a PUT of the key the server has not taken yet, and a map restore that
 * is still running must not bring a forgotten key back.
 */

const MAP = 'items';

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 1;
  binaryType = 'arraybuffer';
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the socket's event payloads have no fixed shape at the test boundary
  onmessage: ((event: { data: any }) => void) | null = null;
  onerror: ((error: unknown) => void) | null = null;
  send = jest.fn();
  close = jest.fn();
  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    queueMicrotask(() => this.onopen?.());
  }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the engine reads WebSocket from the global scope
const originalWebSocket = (globalThis as any).WebSocket;
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- see above
(globalThis as any).WebSocket = MockWebSocket;
afterAll(() => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- see above
  (globalThis as any).WebSocket = originalWebSocket;
});

function deferred() {
  let resolve!: () => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** An in-memory adapter whose `get` of one key can be held, and whose commits can be held or refused. */
class MemoryStorage implements IStorageAdapter {
  kv = new Map<string, unknown>();
  pendingOps: OpLogEntry[] = [];
  private nextId = 1;
  /** While set, the commit waits for this promise (or rejects with it). */
  commitGate: Promise<void> | undefined;
  /** A `get` of this key is held until `releaseGet` is called. */
  holdGet:
    | { key: string; readAfterRelease: boolean; called: ReturnType<typeof deferred> }
    | undefined;
  private release = deferred();

  releaseGet(): void {
    this.release.resolve();
  }

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}

  get = jest.fn(async (key: string) => {
    const hold = this.holdGet;
    if (hold && hold.key === key) {
      this.holdGet = undefined;
      // What a real store returns is read when the call is made; the
      // after-release variant reads it only once the hold ends.
      const captured = this.kv.get(key);
      hold.called.resolve();
      await this.release.promise;
      return hold.readAfterRelease ? this.kv.get(key) : captured;
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the adapter returns stored values of any shape
    return this.kv.get(key) as any;
  });

  put = jest.fn(async (key: string, value: unknown) => {
    this.kv.set(key, value);
  });

  remove = jest.fn(async (key: string) => {
    this.kv.delete(key);
  });

  async getMeta(): Promise<undefined> {
    return undefined;
  }
  async setMeta(): Promise<void> {}
  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [k, v] of entries) this.kv.set(k, v);
  }
  async appendOpLog(): Promise<number> {
    return this.nextId++;
  }
  getPendingOps = jest.fn(async () => this.pendingOps);
  async markOpsSynced(): Promise<void> {}
  async deleteOp(): Promise<void> {}

  commitWrite = jest.fn(async (mutations: StorageMutation[]) => {
    if (this.commitGate) await this.commitGate;
    for (const m of mutations) {
      if (m.store !== 'kv') continue;
      if (m.type === 'remove') this.kv.delete(m.key);
      else this.kv.set(m.key, m.value);
    }
    return this.nextId++;
  });

  getAllKeys = jest.fn(async () => Array.from(this.kv.keys()));
  async getAllMetaKeys(): Promise<string[]> {
    return [];
  }
}

const serverRecord = (value: string | null, millis = Date.now()): LWWRecord<string> => ({
  value,
  timestamp: { millis, counter: 0, nodeId: 'server' },
});

const emptyRoot = () => new MerkleTree().getRootHash();

/** Lets queued microtasks and zero-delay timers run. */
const settle = async (rounds = 5) => {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 0));
};

const until = async (cond: () => boolean, what: string) => {
  for (let i = 0; i < 200; i++) {
    if (cond()) return;
    await new Promise((r) => setTimeout(r, 5));
  }
  throw new Error(`timed out waiting for ${what}`);
};

describe('LWW REMOVE server event (bare engine)', () => {
  let engine: SyncEngine;
  let storage: MemoryStorage;
  let map: LWWMap<string, string>;

  /** Puts a record into memory and storage, as a pulled record would be. */
  const seed = (key: string, record: LWWRecord<string>) => {
    map.merge(key, record);
    storage.kv.set(`${MAP}:${key}`, record);
  };

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the tests drive the engine's private message handlers
  const priv = () => engine as any;

  const removeEvent = (key: string) =>
    priv().handleServerEvent({
      type: 'SERVER_EVENT',
      payload: { mapName: MAP, eventType: 'REMOVE', key },
    });

  const recordPut = (key: string, value: string) => {
    const record = { value, timestamp: engine.getHLC().now() };
    map.merge(key, record);
    return engine.recordOperation(MAP, 'PUT', key, { record, timestamp: record.timestamp }, [
      { store: 'kv', type: 'put', key: `${MAP}:${key}`, value: record },
    ]);
  };

  beforeEach(async () => {
    storage = new MemoryStorage();
    engine = new SyncEngine({
      nodeId: 'client',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: storage,
      heartbeat: { enabled: false },
    });
    await settle();
    map = new LWWMap<string, string>(engine.getHLC());
    engine.registerMap(MAP, map);
  });

  afterEach(() => {
    engine.close();
    jest.restoreAllMocks();
  });

  it('a REMOVE event drops a live key from memory, the Merkle tree and storage, and notifies once', async () => {
    seed('k', serverRecord('v'));
    const listener = jest.fn();
    map.subscribe(listener);

    await removeEvent('k');

    expect(map.get('k')).toBeUndefined();
    expect(map.getRecord('k')).toBeUndefined();
    expect(map.getMerkleTree().getRootHash()).toBe(emptyRoot());
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('a REMOVE event keeps a key with an unsynced PUT in the op log', async () => {
    await recordPut('k', 'local');
    const root = map.getMerkleTree().getRootHash();
    const stored = storage.kv.get(`${MAP}:k`);
    const listener = jest.fn();
    map.subscribe(listener);

    await removeEvent('k');

    expect(map.get('k')).toBe('local');
    expect(map.getMerkleTree().getRootHash()).toBe(root);
    expect(storage.kv.get(`${MAP}:k`)).toBe(stored);
    expect(storage.remove).not.toHaveBeenCalled();
    expect(listener).toHaveBeenCalledTimes(0);
  });

  it('a rejected PUT does not keep the key', async () => {
    await recordPut('k', 'local');
    priv().opLog[0].rejected = true;

    await removeEvent('k');

    expect(map.getRecord('k')).toBeUndefined();
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
  });

  it('a REMOVE event drops the tombstone of a pending local REMOVE and leaves the op queued', async () => {
    seed('k', serverRecord('v', Date.now() - 1_000));
    const tombstone = map.remove('k');
    await engine.recordOperation(
      MAP,
      'REMOVE',
      'k',
      { record: tombstone, timestamp: tombstone.timestamp },
      [{ store: 'kv', type: 'put', key: `${MAP}:k`, value: tombstone }],
    );

    await removeEvent('k');

    expect(map.getRecord('k')).toBeUndefined();
    expect(map.getMerkleTree().getRootHash()).toBe(emptyRoot());
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
    expect(engine.getPendingOpsCount()).toBe(1);
    expect(priv().opLog[0]).toMatchObject({ opType: 'REMOVE', key: 'k', synced: false });
  });

  it('a REMOVE event for an absent key changes nothing in memory and still removes the key from storage', async () => {
    storage.kv.set(`${MAP}:k`, serverRecord('v'));
    const root = map.getMerkleTree().getRootHash();
    const listener = jest.fn();
    map.subscribe(listener);

    await removeEvent('k');

    expect(map.getRecord('k')).toBeUndefined();
    expect(map.getMerkleTree().getRootHash()).toBe(root);
    expect(listener).toHaveBeenCalledTimes(0);
    expect(storage.remove).toHaveBeenCalledTimes(1);
    expect(storage.remove).toHaveBeenCalledWith(`${MAP}:k`);
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
  });

  it('a REMOVE inside SERVER_BATCH_EVENT is applied like a single event', async () => {
    seed('k', serverRecord('v'));
    const put = serverRecord('w');

    await priv().handleServerBatchEvent({
      type: 'SERVER_BATCH_EVENT',
      payload: {
        events: [
          { mapName: MAP, eventType: 'PUT', key: 'other', record: put },
          { mapName: MAP, eventType: 'REMOVE', key: 'k' },
        ],
      },
    });

    expect(map.getRecord('k')).toBeUndefined();
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
    expect(map.get('other')).toBe('w');
  });

  it('a storage removal that fails is logged and does not stop coverage reporting', async () => {
    seed('k', serverRecord('v'));
    storage.remove.mockRejectedValueOnce(new Error('disk gone'));
    const warn = jest.spyOn(logger, 'warn');
    const coverage = jest.spyOn(priv(), 'applyMapCoverage');

    await expect(
      priv().handleServerEvent({
        type: 'SERVER_EVENT',
        payload: { mapName: MAP, eventType: 'REMOVE', key: 'k', epoch: 7 },
      }),
    ).resolves.toBeUndefined();

    expect(map.getRecord('k')).toBeUndefined();
    expect(warn).toHaveBeenCalledWith(
      expect.objectContaining({ mapName: MAP, key: 'k' }),
      expect.any(String),
    );
    expect(coverage).toHaveBeenCalledWith(MAP, 7);
  });
});

describe('LWW REMOVE server event (TopGunClient map)', () => {
  let storage: MemoryStorage;
  let client: TopGunClient;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the tests reach into the client's engine
  const engineOf = (c: TopGunClient): any => (c as any).syncEngine;

  /** Waiters for the end of a map's restore, served by one spy per client. */
  let restoreWaiters: Map<string, () => void>;

  /** Resolves when the restore `getMap(name)` started has ended. */
  const restoreEnd = (name = MAP): Promise<void> => {
    const ended = deferred();
    restoreWaiters.set(name, ended.resolve);
    return ended.promise;
  };

  const build = async () => {
    client = new TopGunClient({ serverUrl: 'ws://localhost:1234', storage });
    restoreWaiters = new Map();
    const engine = engineOf(client);
    const original = engine.endLwwMapRestore.bind(engine);
    jest.spyOn(engine, 'endLwwMapRestore').mockImplementation((name: unknown) => {
      original(name);
      restoreWaiters.get(name as string)?.();
    });
    await settle();
    return client;
  };

  /** Opens the map and waits for its restore to end. */
  const openMap = async () => {
    const ended = restoreEnd();
    const map = client.getMap<string, string>(MAP);
    await ended;
    return map;
  };

  const holdGet = (key: string, readAfterRelease = false) => {
    const called = deferred();
    storage.holdGet = { key: `${MAP}:${key}`, readAfterRelease, called };
    return called.promise;
  };

  const sameStamp = (a: Timestamp | undefined, b: Timestamp) =>
    a !== undefined && HLC.compare(a, b) === 0;

  beforeEach(() => {
    MockWebSocket.instances = [];
    storage = new MemoryStorage();
  });

  afterEach(async () => {
    await client?.close();
    jest.restoreAllMocks();
  });

  it('a REMOVE event keeps a key whose PUT has not yet reached the op log', async () => {
    await build();
    const map = await openMap();
    const engine = engineOf(client);
    const commit = deferred();
    storage.commitGate = commit.promise;

    map.set('k', 'local');
    await settle();
    expect(storage.commitWrite).toHaveBeenCalledTimes(1);
    expect(engine.opLog).toHaveLength(0);

    await engine.applyServerEvent(MAP, 'REMOVE', 'k');
    expect(map.get('k')).toBe('local');

    commit.resolve();
    await settle();
    expect(engine.opLog).toEqual([expect.objectContaining({ opType: 'PUT', key: 'k' })]);
    expect(map.get('k')).toBe('local');
  });

  it('a PUT whose commit failed does not keep the key', async () => {
    await build();
    const map = await openMap();
    const engine = engineOf(client);
    jest.spyOn(logger, 'error').mockImplementation(() => undefined);
    storage.commitGate = Promise.reject(new Error('quota'));
    storage.commitGate.catch(() => undefined);

    map.set('k', 'local');
    await settle();
    expect(engine.opLog).toHaveLength(0);

    await engine.applyServerEvent(MAP, 'REMOVE', 'k');
    expect(map.getRecord('k')).toBeUndefined();
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
  });

  it('a REMOVE event never records an op', async () => {
    await build();
    const map = await openMap();
    const engine = engineOf(client);
    await engine.applyServerEvent(MAP, 'PUT', 'k', serverRecord('v'));
    expect(map.get('k')).toBe('v');
    const removeSpy = jest.spyOn(map, 'remove');
    const recordSpy = jest.spyOn(engine, 'recordOperation');
    const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
    const framesBefore = ws.send.mock.calls.length;
    const opLogBefore = engine.opLog.length;

    await engine.applyServerEvent(MAP, 'REMOVE', 'k');

    expect(map.getRecord('k')).toBeUndefined();
    expect(removeSpy).toHaveBeenCalledTimes(0);
    expect(recordSpy).toHaveBeenCalledTimes(0);
    await settle();
    expect(engine.opLog).toHaveLength(opLogBefore);
    expect(ws.send.mock.calls.length).toBe(framesBefore);
  });

  it('a REMOVE that arrives during the map restore is not undone by the restore', async () => {
    storage.kv.set(`${MAP}:k`, serverRecord('stored'));
    await build();
    const held = holdGet('k');
    const ended = restoreEnd();
    const map = client.getMap<string, string>(MAP);
    await held;

    await engineOf(client).applyServerEvent(MAP, 'REMOVE', 'k');
    storage.releaseGet();
    await ended;

    expect(map.getRecord('k')).toBeUndefined();
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
    expect(map.getMerkleTree().getRootHash()).toBe(
      new LWWMap(new HLC('ref')).getMerkleTree().getRootHash(),
    );
  });

  it('a REMOVE during the restore whose get reads after the removal leaves no key', async () => {
    storage.kv.set(`${MAP}:k`, serverRecord('stored'));
    await build();
    const held = holdGet('k', true);
    const ended = restoreEnd();
    const map = client.getMap<string, string>(MAP);
    await held;

    await engineOf(client).applyServerEvent(MAP, 'REMOVE', 'k');
    storage.releaseGet();
    await ended;

    expect(map.getRecord('k')).toBeUndefined();
    expect(storage.kv.has(`${MAP}:k`)).toBe(false);
    expect(map.getMerkleTree().getRootHash()).toBe(
      new LWWMap(new HLC('ref')).getMerkleTree().getRootHash(),
    );
  });

  it('the restore-scoped set is dropped when the restore ends', async () => {
    await build();
    const engine = engineOf(client);
    await openMap();
    expect(engine.lwwRestoreForgotten.has(MAP)).toBe(false);

    const other = 'other';
    storage.getAllKeys.mockRejectedValueOnce(new Error('read failed'));
    jest.spyOn(logger, 'error').mockImplementation(() => undefined);
    const ended = restoreEnd(other);
    client.getMap<string, string>(other);
    await ended;
    expect(engine.lwwRestoreForgotten.has(other)).toBe(false);

    await engine.applyServerEvent(MAP, 'REMOVE', 'k');
    await engine.applyServerEvent(other, 'REMOVE', 'k');
    expect(engine.lwwRestoreForgotten.size).toBe(0);
  });

  it('the restore does not overwrite a local set whose PUT is pending', async () => {
    storage.kv.set(`${MAP}:k`, serverRecord('stored', Date.now() + 3_600_000));
    await build();
    const engine = engineOf(client);
    const held = holdGet('k');
    const ended = restoreEnd();
    const map = client.getMap<string, string>(MAP);
    await held;

    const local = map.set('k', 'local');
    await until(() => engine.opLog.some((op: OpLogEntry) => op.key === 'k'), 'the op log');
    storage.releaseGet();
    await ended;

    expect(map.get('k')).toBe('local');
    expect(sameStamp(map.getRecord('k')?.timestamp, local.timestamp)).toBe(true);
  });

  it('the restore merges a key whose only pending PUT came from an earlier session', async () => {
    const stored = serverRecord('stored', Date.now() - 1_000);
    stored.timestamp.nodeId = 'earlier-session';
    storage.kv.set(`${MAP}:k`, stored);
    storage.pendingOps = [
      {
        id: 1,
        mapName: MAP,
        opType: 'PUT',
        key: 'k',
        record: stored,
        timestamp: stored.timestamp,
        synced: 0,
      } as unknown as OpLogEntry,
    ];
    await build();
    const engine = engineOf(client);
    const held = holdGet('k');
    const ended = restoreEnd();
    const map = client.getMap<string, string>(MAP);
    await held;
    await until(() => engine.opLog.some((op: OpLogEntry) => op.key === 'k'), 'the op-log load');

    storage.releaseGet();
    await ended;

    expect(map.get('k')).toBe('stored');
    expect(sameStamp(map.getRecord('k')?.timestamp, stored.timestamp)).toBe(true);
  });

  it('the restore merges by stamp when memory holds a record other than the pending PUT', async () => {
    const stored = serverRecord('stored', Date.now() - 1_000);
    stored.timestamp.nodeId = 'earlier-session';
    storage.kv.set(`${MAP}:k`, stored);
    storage.pendingOps = [
      {
        id: 1,
        mapName: MAP,
        opType: 'PUT',
        key: 'k',
        record: stored,
        timestamp: stored.timestamp,
        synced: 0,
      } as unknown as OpLogEntry,
    ];
    await build();
    const engine = engineOf(client);
    const held = holdGet('k');
    const ended = restoreEnd();
    const map = client.getMap<string, string>(MAP);
    await held;
    await until(() => engine.opLog.some((op: OpLogEntry) => op.key === 'k'), 'the op-log load');

    await engine.applyServerEvent(
      MAP,
      'PUT',
      'k',
      serverRecord('server', stored.timestamp.millis - 1_000),
    );
    expect(map.get('k')).toBe('server');
    storage.releaseGet();
    await ended;

    expect(map.get('k')).toBe('stored');
    expect(sameStamp(map.getRecord('k')?.timestamp, stored.timestamp)).toBe(true);
  });
});
