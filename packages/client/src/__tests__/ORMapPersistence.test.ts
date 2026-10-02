import { TopGunClient } from '../TopGunClient';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { HLC, LWWRecord, ORMap, ORMapRecord } from '@topgunbuild/core';
import {
  orMapKeyTombstonesBucketKey,
  orMapKeyTombstonesBucketKeys,
  orMapKeyTombstonesBucketOf,
  OrMapKeyTombstonesWriteHolds,
  restoreOrMapKeyTombstones,
  serializeOrMapKeyTombstonesBucket,
} from '../utils/orMapKeyTombstones';
import { logger } from '../utils/logger';

// Mock Storage Adapter
class MemoryStorageAdapter implements IStorageAdapter {
  private kvStore: Map<string, any> = new Map();
  private metaStore: Map<string, any> = new Map();
  private opLog: OpLogEntry[] = [];
  private _pendingOps: OpLogEntry[] = [];

  async initialize(_dbName: string): Promise<void> {}
  async close(): Promise<void> {}

  async get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined> {
    return this.kvStore.get(key);
  }

  async put(key: string, value: any): Promise<void> {
    this.kvStore.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.kvStore.delete(key);
  }

  async getMeta(key: string): Promise<any> {
    return this.metaStore.get(key);
  }

  async setMeta(key: string, value: any): Promise<void> {
    this.metaStore.set(key, value);
  }

  async batchPut(entries: Map<string, any>): Promise<void> {
    for (const [key, value] of entries) {
      this.kvStore.set(key, value);
    }
  }

  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opLog.length + 1;
    const newEntry = { ...entry, id, synced: 0 };
    this.opLog.push(newEntry);
    this._pendingOps.push(newEntry);
    return id;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    return this._pendingOps;
  }

  async markOpsSynced(lastId: number): Promise<void> {
    this._pendingOps = this._pendingOps.filter((op) => op.id! > lastId);
    this.opLog = this.opLog.filter((op) => op.id! > lastId);
  }

  async deleteOp(id: number): Promise<void> {
    this._pendingOps = this._pendingOps.filter((op) => op.id !== id);
    this.opLog = this.opLog.filter((op) => op.id !== id);
  }

  async commitWrite(
    mutations: Array<{ store: 'kv' | 'meta'; type: 'put' | 'remove'; key: string; value?: any }>,
    op: Omit<OpLogEntry, 'id'>,
  ): Promise<number> {
    for (const m of mutations) {
      const target = m.store === 'meta' ? this.metaStore : this.kvStore;
      if (m.type === 'remove') target.delete(m.key);
      else target.set(m.key, m.value);
    }
    return this.appendOpLog(op);
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kvStore.keys());
  }

  async getAllMetaKeys(): Promise<string[]> {
    return Array.from(this.metaStore.keys());
  }
}

// Install a MockWebSocket for this file. These tests exercise local persistence
// only — they don't need real network behavior. Without a mock, the real undici
// WebSocket dials out and leaves SingleServerProvider's 5s connection-timeout
// (SingleServerProvider.ts:100) pending past each test, keeping Jest's worker
// alive without --forceExit.
const originalWebSocket = (globalThis as any).WebSocket;
(globalThis as any).WebSocket = class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 1;
  binaryType = 'arraybuffer';
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: { data: any }) => void) | null = null;
  onerror: ((error: any) => void) | null = null;
  send = jest.fn();
  close = jest.fn();
  constructor(public url: string) {
    // queueMicrotask, not setTimeout(0): microtasks have no associated timer
    // handle (so they don't appear in jest --detectOpenHandles), and they run
    // before the next macrotask so SingleServerProvider's onopen wrapper
    // (which clears its 5s connection-timeout) runs same-tick as the first
    // awaited operation.
    queueMicrotask(() => {
      if (this.onopen) this.onopen();
    });
  }
};

afterAll(() => {
  (globalThis as any).WebSocket = originalWebSocket;
});

describe('ORMap Integration & Persistence', () => {
  let storage: MemoryStorageAdapter;
  let client: TopGunClient;
  // Track clients constructed inside individual tests so afterEach can dispose
  // them too. An unclosed client keeps a live SingleServerProvider whose
  // heartbeat fails (the mock never pongs) and falls into a reconnect loop;
  // once afterAll restores the real WebSocket, that loop hits real undici and
  // keeps Jest's worker alive indefinitely past the last expect().
  const extraClients: TopGunClient[] = [];

  beforeEach(() => {
    storage = new MemoryStorageAdapter();
    client = new TopGunClient({
      serverUrl: 'ws://localhost:1234',
      storage,
    });
  });

  afterEach(async () => {
    // Dispose the client so its wrapped SyncEngine → SingleServerProvider tears
    // down both the reconnect timer and (via the queueMicrotask onopen path
    // installed above) the 5s connection-timeout. Without this, each test
    // leaks resources that keep Jest's worker alive past the last expect().
    await client.close();
    for (const c of extraClients) {
      await c.close();
    }
    extraClients.length = 0;
  });

  test('should persist added items to storage', async () => {
    const map = client.getORMap<string, string>('tags');
    map.add('list1', 'urgent');
    map.add('list1', 'work');

    // Allow async operations to complete
    await new Promise((resolve) => setTimeout(resolve, 10));

    // Check storage
    const records = await storage.get('tags:list1');
    expect(records).toBeDefined();
    expect(Array.isArray(records)).toBe(true);
    expect(records).toHaveLength(2);

    const values = (records as any[]).map((r) => r.value);
    expect(values).toContain('urgent');
    expect(values).toContain('work');
  });

  test('should persist tombstones (removals) to storage', async () => {
    const map = client.getORMap<string, string>('tags');
    map.add('list1', 'urgent');

    // Allow async operations to complete
    await new Promise((resolve) => setTimeout(resolve, 10));

    map.remove('list1', 'urgent');

    // Allow async operations to complete
    await new Promise((resolve) => setTimeout(resolve, 10));

    // Check KV storage (should be removed if empty or updated)
    // In our impl, if empty we remove the key
    const records = await storage.get('tags:list1');
    expect(records).toBeUndefined();

    // Check Metadata for tombstones
    const tombstones = await storage.getMeta('__sys__:tags:tombstones');
    expect(tombstones).toBeDefined();
    expect(Array.isArray(tombstones)).toBe(true);
    expect(tombstones.length).toBeGreaterThan(0);
  });

  test('should restore ORMap state from storage on initialization', async () => {
    // 1. Setup initial state in storage manually (simulating previous session)
    const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
    const record1 = { value: 'restored_item', timestamp, tag: 'tag1' };

    await storage.put('tags:saved_list', [record1]);

    // 2. Initialize new client/map
    // We need a new client to trigger restore, or just get map again if it wasn't cached (but it caches).
    // Let's make a new client with the SAME storage.
    const newClient = new TopGunClient({
      serverUrl: 'ws://localhost:1234',
      storage,
    });
    extraClients.push(newClient);

    const map = newClient.getORMap<string, string>('tags');

    // 3. Wait for restore (async)
    // Simple polling wait since we don't have a 'ready' event exposed yet
    await new Promise((resolve) => setTimeout(resolve, 50));

    // 4. Verify state
    const values = map.get('saved_list');
    expect(values).toEqual(['restored_item']);
  });

  test('should restore tombstones and respect them', async () => {
    // 1. Setup storage: Item exists in KV but Tombstone exists in Meta
    const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
    const tag = 'deleted_tag';
    const record = { value: 'should_be_deleted', timestamp, tag };

    await storage.put('tags:list', [record]);
    await storage.setMeta('__sys__:tags:tombstones', [tag]);

    // 2. Client load
    const newClient = new TopGunClient({
      serverUrl: 'ws://localhost:1234',
      storage,
    });
    extraClients.push(newClient);

    const map = newClient.getORMap<string, string>('tags');
    await new Promise((resolve) => setTimeout(resolve, 50));

    // 3. Verify: Item should NOT be returned because tag is in tombstones
    const values = map.get('list');
    expect(values).toEqual([]);
  });
});

describe('ORMap per-key tombstone attribution persistence', () => {
  const MAP = 'tags';
  const TOMBSTONES_META = `__sys__:${MAP}:tombstones`;
  // The whole-map entry an earlier, unreleased build wrote. Nothing reads it now.
  const UNBUCKETED_META = `__sys__:${MAP}:keyTombstones`;
  const BUCKET_PREFIX = `${UNBUCKETED_META}:`;

  /** The meta key of the attribution bucket `key` is stored in. */
  const bucketKeyOf = (key: string) =>
    orMapKeyTombstonesBucketKey(MAP, orMapKeyTombstonesBucketOf(key));

  /** A key other than `key` that is stored in the same attribution bucket. */
  const bucketMateOf = (key: string): string => {
    const bucket = orMapKeyTombstonesBucketOf(key);
    for (let i = 0; ; i++) {
      const candidate = `mate-${i}`;
      if (candidate !== key && orMapKeyTombstonesBucketOf(candidate) === bucket) return candidate;
    }
  };

  /** `count` keys other than `key` that are stored in the same attribution bucket. */
  const bucketMatesOf = (key: string, count: number): string[] => {
    const bucket = orMapKeyTombstonesBucketOf(key);
    const mates: string[] = [];
    for (let i = 0; mates.length < count; i++) {
      const candidate = `mate-${i}`;
      if (candidate !== key && orMapKeyTombstonesBucketOf(candidate) === bucket) {
        mates.push(candidate);
      }
    }
    return mates;
  };

  /** The first `count` keys of `key-0, key-1, ...` that fall into `count` different buckets. */
  const keysInDistinctBuckets = (count: number): string[] => {
    const byBucket = new Map<string, string>();
    for (let i = 0; byBucket.size < count; i++) {
      const candidate = `key-${i}`;
      const bucket = orMapKeyTombstonesBucketOf(candidate);
      if (!byBucket.has(bucket)) byBucket.set(bucket, candidate);
    }
    return Array.from(byBucket.values());
  };

  let storage: MemoryStorageAdapter;

  /** Every persisted attribution meta key of the map, bucketed or not. */
  const attributionMetaKeys = async (): Promise<string[]> =>
    (await storage.getAllMetaKeys()).filter((k) => k.startsWith(UNBUCKETED_META)).sort();

  /** The persisted attribution of the whole map: the pairs of all its buckets. */
  const persistedAttribution = async (): Promise<Map<string, string[]>> => {
    const pairs: Array<[string, string[]]> = [];
    for (const metaKey of await attributionMetaKeys()) {
      if (metaKey === UNBUCKETED_META) continue;
      pairs.push(...((await storage.getMeta(metaKey)) as Array<[string, string[]]>));
    }
    return new Map(pairs);
  };
  const clients: TopGunClient[] = [];

  const settle = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
  const sorted = (tags: Iterable<string>) => Array.from(tags).sort();

  // The engine hands the adapter its own op-log entry shape, which the adapter
  // stores opaquely; the adapter's declared entry type does not name `opType`.
  const isRemoveOp = (op: unknown) => (op as { opType?: string }).opType === 'OR_REMOVE';

  function newClient(): TopGunClient {
    const created = new TopGunClient({ serverUrl: 'ws://localhost:1234', storage });
    clients.push(created);
    return created;
  }

  // The two seams that load a persisted OR-Map: the application opening it, and
  // the sync engine instantiating a held map nobody opened this session.
  const restorePaths = [
    [
      'TopGunClient.restoreORMap',
      async (reloaded: TopGunClient) => {
        const map = reloaded.getORMap<string, string>(MAP);
        await settle(50);
        return map;
      },
    ],
    [
      'SyncEngine.instantiateAndRestoreOrMap',
      async (reloaded: TopGunClient) =>
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the second restore seam is private to the engine and only reachable from a sync start
        (await (reloaded as any).syncEngine.instantiateAndRestoreOrMap(MAP)) as ReturnType<
          typeof reloaded.getORMap<string, string>
        >,
    ],
  ] as const;

  beforeEach(() => {
    storage = new MemoryStorageAdapter();
  });

  afterEach(async () => {
    jest.restoreAllMocks();
    for (const c of clients) {
      await c.close();
    }
    clients.length = 0;
  });

  describe('a local remove', () => {
    test('commits the attribution bucket of the removed key in the same batch as the key records and the map-wide tombstones', async () => {
      const map = newClient().getORMap<string, string>(MAP);
      map.add('list1', 'urgent');
      map.add('list1', 'work');
      await settle();

      const commitWrite = jest.spyOn(storage, 'commitWrite');
      const [tag] = map.remove('list1', 'urgent');
      await settle();

      const removeCommits = commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op));
      expect(removeCommits).toHaveLength(1);
      const [mutations] = removeCommits[0];

      const kv = mutations.find((m) => m.store === 'kv' && m.key === `${MAP}:list1`);
      expect(kv?.type).toBe('put');
      expect((kv?.value as Array<{ value: string }>).map((r) => r.value)).toEqual(['work']);

      expect(mutations).toContainEqual({
        store: 'meta',
        type: 'put',
        key: TOMBSTONES_META,
        value: [tag],
      });
      expect(mutations).toContainEqual({
        store: 'meta',
        type: 'put',
        key: bucketKeyOf('list1'),
        value: [['list1', [tag]]],
      });
      // One bucket, and nothing else of the attribution, travels with the remove.
      expect(mutations.filter((m) => m.key.startsWith(UNBUCKETED_META))).toHaveLength(1);
      expect(mutations).toHaveLength(3);
    });

    test('commits the whole current bucket, including every tag of a multi-tag remove, with its first op', async () => {
      const mate = bucketMateOf('list1');
      const map = newClient().getORMap<string, string>(MAP);
      map.add(mate, 'x');
      map.add('list1', 'dup');
      map.add('list1', 'dup');
      await settle();
      const [mateTag] = map.remove(mate, 'x');
      await settle();

      const commitWrite = jest.spyOn(storage, 'commitWrite');
      const tags = map.remove('list1', 'dup');
      expect(tags).toHaveLength(2);
      await settle();

      // Two OR_REMOVE ops, one atomic commit: the second tag is an op-only append.
      const removeCommits = commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op));
      expect(removeCommits).toHaveLength(1);
      const attribution = removeCommits[0][0].find((m) => m.key === bucketKeyOf('list1'));
      const persisted = new Map(attribution?.value as Array<[string, string[]]>);
      expect(sorted(persisted.keys())).toEqual(sorted(['list1', mate]));
      expect(persisted.get(mate)).toEqual([mateTag]);
      expect(sorted(persisted.get('list1') ?? [])).toEqual(sorted(tags));
    });

    test('commits one bucket holding only the keys of that bucket, and leaves the other buckets as they were', async () => {
      // Attributed keys in five buckets, one of them shared by two keys.
      const spread = keysInDistinctBuckets(5);
      const [target] = spread;
      const mate = bucketMateOf(target);
      const map = newClient().getORMap<string, string>(MAP);
      for (const key of [...spread, mate]) {
        map.add(key, 'first');
        map.add(key, 'second');
      }
      await settle();
      for (const key of [...spread.slice(1), mate]) {
        map.remove(key, 'first');
      }
      await settle();
      const others = spread.slice(1);
      const otherBucketsBefore = await Promise.all(
        others.map((k) => storage.getMeta(bucketKeyOf(k))),
      );
      for (const [i, key] of others.entries()) {
        expect((otherBucketsBefore[i] as Array<[string, string[]]>).map(([k]) => k)).toEqual([key]);
      }

      const commitWrite = jest.spyOn(storage, 'commitWrite');
      const setMeta = jest.spyOn(storage, 'setMeta');
      const [tag] = map.remove(target, 'first');
      await settle();

      const removeCommits = commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op));
      expect(removeCommits).toHaveLength(1);
      const [mutations] = removeCommits[0];
      const attribution = mutations.filter((m) => m.key.startsWith(UNBUCKETED_META));
      expect(attribution).toHaveLength(1);
      expect(attribution[0].key).toBe(bucketKeyOf(target));
      const committed = new Map(attribution[0].value as Array<[string, string[]]>);
      expect(sorted(committed.keys())).toEqual(sorted([target, mate]));
      expect(committed.get(target)).toEqual([tag]);
      // The same batch carries the key's records and the map-wide tombstones.
      expect(mutations.map((m) => m.key).sort()).toEqual(
        [`${MAP}:${target}`, TOMBSTONES_META, bucketKeyOf(target)].sort(),
      );
      // Nothing else wrote attribution, and the other buckets are untouched.
      expect(setMeta).not.toHaveBeenCalled();
      expect(await Promise.all(others.map((k) => storage.getMeta(bucketKeyOf(k))))).toEqual(
        otherBucketsBefore,
      );
    });
  });

  describe('a local remove whose commit waits behind backpressure', () => {
    test('commits the state at commit time, so a server response written during the wait is not overwritten', async () => {
      const mate = bucketMateOf('K');
      const client = newClient();
      const map = client.getORMap<string, string>(MAP);
      map.add('K', 'removed');
      map.add('K', 'kept');
      map.add(mate, 'live');
      await settle();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test holds the engine's backpressure wait open and injects a server response into its sync handler
      const engine = (client as any).syncEngine;

      let release: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      jest.spyOn(engine.backpressureController, 'checkBackpressure').mockReturnValue(gate);
      const commitWrite = jest.spyOn(storage, 'commitWrite');

      const [tag] = map.remove('K', 'removed');
      await settle();
      // The remove is held before its commit: nothing of it is on disk yet.
      expect(commitWrite).not.toHaveBeenCalled();
      expect(await storage.getMeta(bucketKeyOf('K'))).toBeUndefined();

      // While it waits, the server attributes a tombstone to another key of the
      // same bucket and announces a further record for the removed key itself.
      const arrived = {
        value: 'arrived',
        tag: 'server-record-tag',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'server' },
      };
      await engine.orMapSyncHandler.handleORMapDiffResponse({
        mapName: MAP,
        entries: [{ key: mate, records: map.getRecords(mate), tombstones: ['server-tag'] }],
      });
      await engine.applyServerEvent(MAP, 'OR_ADD', 'K', undefined, arrived);
      const bucketFromServer = new Map(
        (await storage.getMeta(bucketKeyOf('K'))) as Array<[string, string[]]>,
      );
      expect(bucketFromServer.get(mate)).toEqual(['server-tag']);
      expect(bucketFromServer.get('K')).toEqual([tag]);

      release();
      await settle();

      expect(commitWrite).toHaveBeenCalledTimes(1);
      const [mutations] = commitWrite.mock.calls[0];
      const committedBucket = new Map(
        mutations.find((m) => m.key === bucketKeyOf('K'))?.value as Array<[string, string[]]>,
      );
      expect(committedBucket.get(mate)).toEqual(['server-tag']);
      expect(committedBucket.get('K')).toEqual([tag]);
      // The other two values of the same commit are as current as the bucket.
      const committedRecords = mutations.find((m) => m.key === `${MAP}:K`)?.value as Array<{
        value: string;
      }>;
      expect(sorted(committedRecords.map((r) => r.value))).toEqual(['arrived', 'kept']);
      expect(sorted(mutations.find((m) => m.key === TOMBSTONES_META)?.value as string[])).toEqual(
        sorted(['server-tag', tag]),
      );

      // And so is what storage holds afterwards.
      const stored = new Map(
        (await storage.getMeta(bucketKeyOf('K'))) as Array<[string, string[]]>,
      );
      expect(stored.get(mate)).toEqual(['server-tag']);
      expect(stored.get('K')).toEqual([tag]);
      expect(
        sorted(((await storage.get(`${MAP}:K`)) as Array<{ value: string }>).map((r) => r.value)),
      ).toEqual(['arrived', 'kept']);
    });

    test('commits its own tags in the bucket even when a server response dropped them from the key during the wait', async () => {
      const client = newClient();
      const map = client.getORMap<string, string>(MAP);
      map.add('K', 'removed');
      map.add('K', 'removed');
      map.add('K', 'kept');
      await settle();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test holds the engine's backpressure wait open and injects a server response into its sync handler
      const engine = (client as any).syncEngine;

      let release: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      jest.spyOn(engine.backpressureController, 'checkBackpressure').mockReturnValue(gate);
      const commitWrite = jest.spyOn(storage, 'commitWrite');

      const tags = map.remove('K', 'removed');
      expect(tags).toHaveLength(2);
      await settle();
      expect(commitWrite).not.toHaveBeenCalled();

      // While the remove waits, the server reports the key as it holds it: the
      // kept record and no tombstone. The remove is not in the op log yet, so
      // nothing marks its tags as pending and the key's attribution is replaced
      // by the server's empty set.
      await engine.orMapSyncHandler.handleORMapDiffResponse({
        mapName: MAP,
        entries: [{ key: 'K', records: map.getRecords('K'), tombstones: [] }],
      });
      expect(map.getKeyTombstones('K').size).toBe(0);

      release();
      await settle();

      // The commit that makes the removes durable carries their attribution,
      // whatever the key's attribution in memory was reduced to in between.
      const removeCommits = commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op));
      expect(removeCommits).toHaveLength(1);
      const committedBucket = new Map(
        removeCommits[0][0].find((m) => m.key === bucketKeyOf('K'))?.value as Array<
          [string, string[]]
        >,
      );
      expect(sorted(committedBucket.get('K') ?? [])).toEqual(sorted(tags));
      const stored = new Map(
        (await storage.getMeta(bucketKeyOf('K'))) as Array<[string, string[]]>,
      );
      expect(sorted(stored.get('K') ?? [])).toEqual(sorted(tags));
    });
  });

  describe('a local remove that commits while the map is being reset', () => {
    test('does not refill a bucket the reset has already emptied', async () => {
      const keys = keysInDistinctBuckets(2);
      const client = newClient();
      const map = client.getORMap<string, string>(MAP);
      for (const key of keys) {
        map.add(key, 'gone');
        map.add(key, 'kept');
      }
      await settle();
      for (const key of keys) map.remove(key, 'gone');
      await settle();
      for (const key of keys) {
        expect((await storage.getMeta(bucketKeyOf(key))) as unknown[]).toHaveLength(1);
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the full-resync REPLACE is private to the engine and only reachable from a sync response
      const engine = (client as any).syncEngine;

      // Hold the reset at the second bucket it empties: the first is then
      // already empty on disk while the map in memory is not yet cleared.
      let releaseReset: () => void = () => undefined;
      const resetGate = new Promise<void>((resolve) => {
        releaseReset = resolve;
      });
      let bucketsEmptied = 0;
      const setMeta = storage.setMeta.bind(storage);
      jest.spyOn(storage, 'setMeta').mockImplementation(async (key, value) => {
        const emptiesBucket =
          key.startsWith(BUCKET_PREFIX) && Array.isArray(value) && value.length === 0;
        if (emptiesBucket && ++bucketsEmptied === 2) await resetGate;
        return setMeta(key, value);
      });

      const replaced = engine.replaceOrMapFromSnapshot(MAP, undefined);
      await settle();
      const emptied: string[] = [];
      for (const key of keys) {
        if (((await storage.getMeta(bucketKeyOf(key))) as unknown[]).length === 0) {
          emptied.push(key);
        }
      }
      expect(emptied).toHaveLength(1);

      // A local remove of a key in the bucket that is already emptied commits now.
      const commitWrite = jest.spyOn(storage, 'commitWrite');
      expect(map.remove(emptied[0], 'kept')).toHaveLength(1);
      await settle();
      expect(commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op))).toHaveLength(1);

      releaseReset();
      await replaced;
      await settle();

      // The map is wiped, and nothing on disk attributes a tombstone any more.
      expect(map.getSnapshot().keyTombstones.size).toBe(0);
      expect(sorted((await persistedAttribution()).keys())).toEqual([]);
    });
  });

  describe('a local remove that commits while a reset is running that then fails', () => {
    test('has its bucket written after all, from the map the failed reset left intact', async () => {
      const keys = keysInDistinctBuckets(2);
      const client = newClient();
      const map = client.getORMap<string, string>(MAP);
      for (const key of keys) {
        map.add(key, 'gone');
        map.add(key, 'kept');
      }
      await settle();
      const goneTags = new Map(keys.map((key) => [key, map.remove(key, 'gone')[0]]));
      await settle();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the full-resync REPLACE is private to the engine and only reachable from a sync response
      const engine = (client as any).syncEngine;

      // The reset empties one bucket, is held at the second, and fails there.
      let failReset: () => void = () => undefined;
      const resetGate = new Promise<void>((resolve) => {
        failReset = resolve;
      });
      let bucketsEmptied = 0;
      const setMeta = storage.setMeta.bind(storage);
      const setMetaSpy = jest.spyOn(storage, 'setMeta').mockImplementation(async (key, value) => {
        const emptiesBucket =
          key.startsWith(BUCKET_PREFIX) && Array.isArray(value) && value.length === 0;
        if (emptiesBucket && ++bucketsEmptied === 2) {
          await resetGate;
          throw new Error('simulated durable setMeta failure');
        }
        return setMeta(key, value);
      });
      jest.spyOn(logger, 'error').mockImplementation(() => undefined);

      const replaced = engine.replaceOrMapFromSnapshot(MAP, undefined);
      const outcome = replaced.then(
        () => 'resolved',
        (err: Error) => err.message,
      );
      await settle();
      const emptiedKeys: string[] = [];
      for (const key of keys) {
        if (((await storage.getMeta(bucketKeyOf(key))) as unknown[]).length === 0) {
          emptiedKeys.push(key);
        }
      }
      expect(emptiedKeys).toHaveLength(1);
      const target = emptiedKeys[0];

      const [keptTag] = map.remove(target, 'kept');
      await settle();
      // Left out of the commit while the reset runs.
      expect(await storage.getMeta(bucketKeyOf(target))).toEqual([]);

      failReset();
      expect(await outcome).toBe('simulated durable setMeta failure');
      setMetaSpy.mockRestore();
      await settle();

      // The reset failed closed: the map still holds everything, and the bucket
      // the remove could not commit is on disk with what memory attributes.
      for (const key of keys) {
        expect(map.getKeyTombstones(key).has(goneTags.get(key) as string)).toBe(true);
      }
      const onDisk = new Map(
        (await storage.getMeta(bucketKeyOf(target))) as Array<[string, string[]]>,
      );
      expect(sorted(onDisk.get(target) ?? [])).toEqual(
        sorted([goneTags.get(target) as string, keptTag]),
      );
    });
  });

  describe('attribution write holds', () => {
    test('a map is held until every overlapping hold has ended, and only then are the deferred buckets handed back', async () => {
      const holds = new OrMapKeyTombstonesWriteHolds();
      expect(holds.isHeld('m')).toBe(false);
      await holds.whenReleased('m');

      holds.begin('m');
      holds.begin('m');
      holds.defer('m', 'ab');
      holds.defer('m', 'ab');
      holds.defer('m', 'cd');
      // Another map is not affected, and deferring for it records nothing.
      expect(holds.isHeld('other')).toBe(false);
      holds.defer('other', 'ef');
      expect(holds.end('other', true)).toEqual([]);

      let released = false;
      void holds.whenReleased('m').then(() => {
        released = true;
      });

      expect(holds.end('m', true)).toEqual([]);
      await Promise.resolve();
      expect(holds.isHeld('m')).toBe(true);
      expect(released).toBe(false);

      expect(holds.end('m', true).sort()).toEqual(['ab', 'cd']);
      expect(holds.isHeld('m')).toBe(false);
      await Promise.resolve();
      expect(released).toBe(true);
      // Ending a hold that is not there is harmless.
      expect(holds.end('m', true)).toEqual([]);
    });

    test('a holder that ends with nothing worth writing discards what was deferred up to then', () => {
      const holds = new OrMapKeyTombstonesWriteHolds();
      holds.begin('m');
      holds.begin('m');
      holds.defer('m', 'ab');
      expect(holds.end('m', false)).toEqual([]);
      holds.defer('m', 'cd');
      expect(holds.end('m', true)).toEqual(['cd']);
    });
  });

  describe('the bucket value committed with a local remove', () => {
    test('lists the remove tags under its key once, whether the map attributes all, some or none of them', () => {
      const map = new ORMap<string, string>(new HLC('n1'));
      const mate = bucketMateOf('K');
      const bucket = orMapKeyTombstonesBucketOf('K');
      map.addKeyTombstones([
        [mate, ['mate-tag']],
        ['K', ['t1']],
      ]);

      const some = new Map(
        serializeOrMapKeyTombstonesBucket(map, bucket, { key: 'K', tags: ['t1', 't2'] }),
      );
      expect(some.get('K')).toEqual(['t1', 't2']);
      expect(some.get(mate)).toEqual(['mate-tag']);

      const all = new Map(
        serializeOrMapKeyTombstonesBucket(map, bucket, { key: 'K', tags: ['t1'] }),
      );
      expect(all.get('K')).toEqual(['t1']);

      map.setKeyTombstones('K', []);
      const none = serializeOrMapKeyTombstonesBucket(map, bucket, { key: 'K', tags: ['t1', 't2'] });
      expect(none.filter(([key]) => key === 'K')).toEqual([['K', ['t1', 't2']]]);

      // The map itself is only read.
      expect(map.getKeyTombstones('K').size).toBe(0);
      // Without a remove, and for a remove that took no tag, the value is the map's own.
      expect(serializeOrMapKeyTombstonesBucket(map, bucket)).toEqual([[mate, ['mate-tag']]]);
      expect(serializeOrMapKeyTombstonesBucket(map, bucket, { key: 'K', tags: [] })).toEqual([
        [mate, ['mate-tag']],
      ]);
    });
  });

  describe('bucket layout', () => {
    /** The trie path of the leaf holding `key`, read off the map's own Merkle tree. */
    const leafPathOf = (map: ORMap<string, string>, key: string): string => {
      const tree = map.getMerkleTree();
      const search = (path: string): string | undefined => {
        if (tree.getKeysInBucket(path).includes(key)) return path;
        for (const child of Object.keys(tree.getBuckets(path))) {
          const found = search(path + child);
          if (found !== undefined) return found;
        }
        return undefined;
      };
      const path = search('');
      if (path === undefined) throw new Error(`key ${key} is not in the tree`);
      return path;
    };

    test('a key is stored in the bucket named by the first two characters of its Merkle path', () => {
      const map = new ORMap<string, string>(new HLC('n1'));
      const keys = ['K', '__proto__', '', 'ключ', '🙂', ...keysInDistinctBuckets(40)];
      for (const key of keys) {
        map.add(key, 'v');
        const bucket = orMapKeyTombstonesBucketOf(key);
        expect(bucket).toMatch(/^[0-9a-f]{2}$/);
        expect(bucket).toBe(leafPathOf(map, key).slice(0, 2));
        expect(orMapKeyTombstonesBucketKey(MAP, bucket)).toBe(
          `__sys__:${MAP}:keyTombstones:${bucket}`,
        );
      }
    });

    test('only keys made of the map prefix and one bucket id are listed as its buckets', () => {
      const metaKeys = [
        '__sys__:m:keyTombstones:00',
        '__sys__:m:keyTombstones:ff',
        // The un-bucketed entry of an unreleased build.
        '__sys__:m:keyTombstones',
        // Not bucket ids.
        '__sys__:m:keyTombstones:',
        '__sys__:m:keyTombstones:0',
        '__sys__:m:keyTombstones:000',
        '__sys__:m:keyTombstones:FF',
        '__sys__:m:keyTombstones:zz',
        // Other reserved keys of the map.
        '__sys__:m:ormap',
        '__sys__:m:tombstones',
        // Buckets of maps whose names extend or are extended by this one.
        '__sys__:m:keyTombstones:keyTombstones:ab',
        '__sys__:mm:keyTombstones:ab',
        '__sys__:m:x:keyTombstones:ab',
      ];

      expect(orMapKeyTombstonesBucketKeys('m', metaKeys)).toEqual([
        '__sys__:m:keyTombstones:00',
        '__sys__:m:keyTombstones:ff',
      ]);
      expect(orMapKeyTombstonesBucketKeys('m:keyTombstones', metaKeys)).toEqual([
        '__sys__:m:keyTombstones:keyTombstones:ab',
      ]);
      expect(orMapKeyTombstonesBucketKeys('m:x', metaKeys)).toEqual([
        '__sys__:m:x:keyTombstones:ab',
      ]);
      expect(orMapKeyTombstonesBucketKeys('absent', metaKeys)).toEqual([]);
    });
  });

  describe.each(restorePaths)('reload through %s', (_name, reload) => {
    test('restores the attribution, so the Merkle root equals the root before the reload', async () => {
      const map = newClient().getORMap<string, string>(MAP);
      // Further keys, so that the attribution is spread over several buckets and
      // one of them holds two keys.
      const spread = [...keysInDistinctBuckets(4), bucketMateOf('gone')];
      map.add('gone', 'x');
      map.add('mixed', 'keep');
      map.add('mixed', 'drop');
      for (const key of spread) {
        map.add(key, 'keep');
        map.add(key, 'drop');
      }
      await settle();
      // `gone` ends up holding tombstones only; `mixed` holds a live record and
      // an attributed tombstone. Both leaves depend on the attribution.
      const [goneTag] = map.remove('gone', 'x');
      const [dropTag] = map.remove('mixed', 'drop');
      const spreadTags = new Map(spread.map((key) => [key, map.remove(key, 'drop')]));
      await settle();

      const nonEmptyBuckets: string[] = [];
      for (const metaKey of await attributionMetaKeys()) {
        if (((await storage.getMeta(metaKey)) as unknown[]).length > 0)
          nonEmptyBuckets.push(metaKey);
      }
      expect(nonEmptyBuckets.length).toBeGreaterThanOrEqual(4);
      expect(sorted((await persistedAttribution()).keys())).toEqual(
        sorted(['gone', 'mixed', ...spread]),
      );

      const rootBefore = map.getMerkleTree().getRootHash();
      expect(rootBefore).not.toBe(0);
      expect(map.get('gone')).toEqual([]);

      const restored = await reload(newClient());

      expect(restored.getMerkleTree().getRootHash()).toBe(rootBefore);
      expect(sorted(restored.getKeyTombstones('gone'))).toEqual([goneTag]);
      expect(sorted(restored.getKeyTombstones('mixed'))).toEqual([dropTag]);
      expect(restored.get('mixed')).toEqual(['keep']);
      for (const key of spread) {
        expect(sorted(restored.getKeyTombstones(key))).toEqual(sorted(spreadTags.get(key) ?? []));
        expect(restored.get(key)).toEqual(['keep']);
      }
    });

    test('reads the buckets through one listing of the meta keys and applies them in one pass', async () => {
      const keys = keysInDistinctBuckets(6);
      const map = newClient().getORMap<string, string>(MAP);
      for (const key of keys) map.add(key, 'v');
      await settle();
      for (const key of keys) map.remove(key, 'v');
      await settle();
      const rootBefore = map.getMerkleTree().getRootHash();

      const reloaded = newClient();
      // Let the new client's own start-up reads finish before counting.
      await settle(50);
      const getAllMetaKeys = jest.spyOn(storage, 'getAllMetaKeys');
      const addKeyTombstones = jest.spyOn(ORMap.prototype, 'addKeyTombstones');
      const restored = await reload(reloaded);

      expect(restored.getMerkleTree().getRootHash()).toBe(rootBefore);
      expect(getAllMetaKeys).toHaveBeenCalledTimes(1);
      expect(addKeyTombstones).toHaveBeenCalledTimes(1);
      const [pairs] = addKeyTombstones.mock.calls[0];
      expect(sorted(Array.from(pairs, ([key]) => key as string))).toEqual(sorted(keys));
    });

    test('a key named __proto__ round-trips', async () => {
      const prototypeKeysBefore = Object.getOwnPropertyNames(Object.prototype).sort();
      const map = newClient().getORMap<string, string>(MAP);
      map.add('__proto__', 'v');
      await settle();
      const [tag] = map.remove('__proto__', 'v');
      await settle();

      expect(await storage.getMeta(bucketKeyOf('__proto__'))).toEqual([['__proto__', [tag]]]);
      const rootBefore = map.getMerkleTree().getRootHash();

      const restored = await reload(newClient());

      expect(restored.getMerkleTree().getRootHash()).toBe(rootBefore);
      expect(sorted(restored.getKeyTombstones('__proto__'))).toEqual([tag]);
      expect(Object.getOwnPropertyNames(Object.prototype).sort()).toEqual(prototypeKeysBefore);
      expect(Object.getPrototypeOf({})).toBe(Object.prototype);
    });

    test('a store with map-wide tombstones but no attribution entry loads with no attribution and still suppresses', async () => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      const removed = { value: 'should_be_deleted', timestamp, tag: 'deleted_tag' };
      const live = { value: 'live', timestamp, tag: 'live_tag' };
      await storage.put(`${MAP}:list`, [removed, live]);
      await storage.setMeta(TOMBSTONES_META, ['deleted_tag', 'orphan_tag']);

      const restored = await reload(newClient());

      expect(restored.getSnapshot().keyTombstones.size).toBe(0);
      expect(restored.getKeyTombstones('list').size).toBe(0);
      expect(sorted(restored.getTombstones())).toEqual(['deleted_tag', 'orphan_tag']);
      expect(restored.get('list')).toEqual(['live']);
      // Remove-wins: a record carrying a tombstoned tag is refused on arrival too.
      expect(restored.apply('list', removed)).toBe(false);
      expect(restored.apply('elsewhere', { value: 'late', timestamp, tag: 'orphan_tag' })).toBe(
        false,
      );
      expect(restored.get('elsewhere')).toEqual([]);
      // Loading wrote nothing: the store is still one without an attribution entry.
      expect(await attributionMetaKeys()).toEqual([]);
    });

    test('the un-bucketed entry of an unreleased build is neither read nor an error', async () => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      await storage.put(`${MAP}:list`, [{ value: 'live', timestamp, tag: 'live_tag' }]);
      await storage.put(`${MAP}:elsewhere`, [{ value: 'kept', timestamp, tag: 'stale_tag' }]);
      await storage.setMeta(UNBUCKETED_META, [['list', ['stale_tag']]]);
      const getMeta = jest.spyOn(storage, 'getMeta');
      const warn = jest.spyOn(logger, 'warn').mockImplementation(() => undefined);
      const error = jest.spyOn(logger, 'error').mockImplementation(() => undefined);

      const restored = await reload(newClient());

      expect(restored.getSnapshot().keyTombstones.size).toBe(0);
      expect(restored.getTombstones()).not.toContain('stale_tag');
      expect(restored.get('list')).toEqual(['live']);
      expect(restored.get('elsewhere')).toEqual(['kept']);
      expect(getMeta).not.toHaveBeenCalledWith(UNBUCKETED_META);
      expect(warn).not.toHaveBeenCalled();
      expect(error).not.toHaveBeenCalled();
      // It is left alone, not rewritten or migrated.
      expect(await storage.getMeta(UNBUCKETED_META)).toEqual([['list', ['stale_tag']]]);
      expect(await attributionMetaKeys()).toEqual([UNBUCKETED_META]);
    });

    test('a persisted record whose tag is attributed but missing from the map-wide tombstones stays removed', async () => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      await storage.put(`${MAP}:elsewhere`, [{ value: 'stale', timestamp, tag: 'moved_tag' }]);
      await storage.setMeta(TOMBSTONES_META, []);
      await storage.setMeta(bucketKeyOf('list'), [['list', ['moved_tag']]]);

      const restored = await reload(newClient());

      expect(restored.get('elsewhere')).toEqual([]);
      expect(restored.getTombstones()).toContain('moved_tag');
      expect(sorted(restored.getKeyTombstones('list'))).toEqual(['moved_tag']);
    });

    test.each([
      ['a non-array bucket', 'garbage'],
      ['an object keyed by map key', { list: ['t1'] }],
      ['null', null],
    ])('%s is ignored and the map still loads', async (_shape, persisted) => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      await storage.put(`${MAP}:list`, [{ value: 'live', timestamp, tag: 'live_tag' }]);
      await storage.setMeta(bucketKeyOf('list'), persisted);

      const restored = await reload(newClient());

      expect(restored.get('list')).toEqual(['live']);
      expect(restored.getSnapshot().keyTombstones.size).toBe(0);
    });

    test('an unreadable bucket does not keep the readable ones from being restored', async () => {
      const [damaged, intact] = keysInDistinctBuckets(2);
      await storage.setMeta(bucketKeyOf(damaged), 'garbage');
      await storage.setMeta(bucketKeyOf(intact), [[intact, ['t1']]]);

      const restored = await reload(newClient());

      expect(sorted(restored.getSnapshot().keyTombstones.keys())).toEqual([intact]);
      expect(sorted(restored.getKeyTombstones(intact))).toEqual(['t1']);
    });

    test('a bucket whose read fails is reported and does not keep the readable ones from being restored', async () => {
      const [failing, intactA, intactB] = keysInDistinctBuckets(3);
      for (const key of [failing, intactA, intactB]) {
        await storage.setMeta(bucketKeyOf(key), [[key, [`tag-of-${key}`]]]);
      }
      const getMeta = storage.getMeta.bind(storage);
      jest.spyOn(storage, 'getMeta').mockImplementation(async (key) => {
        if (key === bucketKeyOf(failing)) throw new Error('simulated bucket read failure');
        return getMeta(key);
      });
      const warn = jest.spyOn(logger, 'warn').mockImplementation(() => undefined);
      jest.spyOn(logger, 'error').mockImplementation(() => undefined);

      const restored = await reload(newClient());

      expect(sorted(restored.getSnapshot().keyTombstones.keys())).toEqual(
        sorted([intactA, intactB]),
      );
      for (const key of [intactA, intactB]) {
        expect(sorted(restored.getKeyTombstones(key))).toEqual([`tag-of-${key}`]);
      }
      // One warning for the one bucket that could not be read, naming it.
      const reported = warn.mock.calls.filter(
        ([context]) => (context as { bucketKey?: string })?.bucketKey === bucketKeyOf(failing),
      );
      expect(reported).toHaveLength(1);
    });

    test('malformed pairs are skipped one by one; the readable ones are restored', async () => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      await storage.put(`${MAP}:list`, [{ value: 'live', timestamp, tag: 'live_tag' }]);
      await storage.setMeta(bucketKeyOf('good'), [
        'not-a-pair',
        null,
        ['only-a-key'],
        [42, ['numeric-key']],
        ['tags-not-an-array', 'oops'],
        ['empty', []],
        ['good', ['t1', 7, 't2']],
      ]);

      const restored = await reload(newClient());

      expect(restored.get('list')).toEqual(['live']);
      expect(sorted(restored.getSnapshot().keyTombstones.keys())).toEqual(['good']);
      expect(sorted(restored.getKeyTombstones('good'))).toEqual(['t1', 't2']);
      expect(sorted(restored.getTombstones())).toEqual(['t1', 't2']);
    });
  });

  describe('unreadable persisted attribution is reported, not skipped silently', () => {
    let warn: jest.SpyInstance;

    beforeEach(() => {
      warn = jest.spyOn(logger, 'warn').mockImplementation(() => undefined);
    });

    afterEach(() => {
      warn.mockRestore();
    });

    test('every skipped pair and tag is counted in one warning', () => {
      const map = new ORMap<string, string>(new HLC('n1'));

      restoreOrMapKeyTombstones(map, [
        [
          'not-a-pair',
          null,
          ['only-a-key'],
          [42, ['numeric-key']],
          ['tags-not-an-array', 'oops'],
          ['empty', []],
          ['good', ['t1', 7, 't2']],
        ],
      ]);

      expect(sorted(map.getKeyTombstones('good'))).toEqual(['t1', 't2']);
      expect(warn).toHaveBeenCalledTimes(1);
      expect(warn.mock.calls[0][0]).toEqual({ skipped: 6 });
    });

    test('what is skipped in several buckets is still one warning, with the total', () => {
      const map = new ORMap<string, string>(new HLC('n1'));

      restoreOrMapKeyTombstones(map, [
        ['not-a-pair', ['a', ['t1', 7]]],
        'not-a-bucket',
        undefined,
        [
          ['b', ['t2']],
          ['tags-not-an-array', 'oops'],
        ],
        { c: ['t3'] },
      ]);

      expect(sorted(map.getKeyTombstones('a'))).toEqual(['t1']);
      expect(sorted(map.getKeyTombstones('b'))).toEqual(['t2']);
      expect(sorted(map.getSnapshot().keyTombstones.keys())).toEqual(['a', 'b']);
      expect(warn).toHaveBeenCalledTimes(1);
      // One pair and one tag in the first bucket, the second bucket as a whole,
      // one pair in the fourth, the fifth bucket as a whole.
      expect(warn.mock.calls[0][0]).toEqual({ skipped: 5 });
    });

    test('a bucket that is not a list of pairs is reported', () => {
      const map = new ORMap<string, string>(new HLC('n1'));

      restoreOrMapKeyTombstones(map, [{ K: ['t1'] }]);

      expect(map.getSnapshot().keyTombstones.size).toBe(0);
      expect(warn).toHaveBeenCalledTimes(1);
    });

    test.each([
      ['no bucket at all', []],
      ['an absent bucket', [undefined]],
      ['an empty bucket', [[]]],
      ['a readable bucket', [[['K', ['t1']]]]],
    ])('%s is not reported', (_shape, buckets) => {
      const map = new ORMap<string, string>(new HLC('n1'));

      restoreOrMapKeyTombstones(map, buckets);

      expect(warn).not.toHaveBeenCalled();
    });
  });

  describe('a remove that lands while the map is still loading', () => {
    test('keeps its attribution when the persisted one is applied afterwards', () => {
      // The application-facing load is not awaited, so a remove can attribute its
      // tag before the persisted attribution for the same key is read back.
      const map = new ORMap<string, string>(new HLC('n1'));
      map.add('K', 'v');
      const [fresh] = map.remove('K', 'v');

      restoreOrMapKeyTombstones(map, [[['K', ['persisted']]], [['other', ['elsewhere']]]]);

      expect(sorted(map.getKeyTombstones('K'))).toEqual(sorted([fresh, 'persisted']));
      expect(sorted(map.getKeyTombstones('other'))).toEqual(['elsewhere']);
      expect(sorted(map.getTombstones())).toEqual(sorted([fresh, 'persisted', 'elsewhere']));
    });
  });

  describe('an attribution write that lands while the map is still loading', () => {
    /**
     * Storage as a first session leaves it: two keys of one bucket hold an
     * attributed tombstone each, and a third key of that bucket (`K`) holds a
     * live record.
     */
    const firstSession = async () => {
      const [k1, k2] = bucketMatesOf('K', 2);
      const map = newClient().getORMap<string, string>(MAP);
      map.add(k1, 'gone');
      map.add(k2, 'gone');
      const live = map.add('K', 'live');
      await settle();
      const [t1] = map.remove(k1, 'gone');
      const [t2] = map.remove(k2, 'gone');
      await settle();
      expect(
        new Map((await storage.getMeta(bucketKeyOf('K'))) as Array<[string, string[]]>),
      ).toEqual(
        new Map([
          [k1, [t1]],
          [k2, [t2]],
        ]),
      );
      return { k1, k2, t1, t2, liveTag: live.tag };
    };

    /**
     * Holds every listing of the meta keys open. A load lists them once, after
     * the records and before it reads the attribution buckets, so the map then
     * holds its records but none of its persisted attribution.
     */
    const holdAttributionLoad = (): (() => void) => {
      let release: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      const getAllMetaKeys = storage.getAllMetaKeys.bind(storage);
      jest.spyOn(storage, 'getAllMetaKeys').mockImplementation(async () => {
        await gate;
        return getAllMetaKeys();
      });
      return release;
    };

    const bucketOnDisk = async (): Promise<Map<string, string[]>> =>
      new Map((await storage.getMeta(bucketKeyOf('K'))) as Array<[string, string[]]>);

    test('a local remove before the load finishes leaves all three keys attributed on disk and in memory', async () => {
      const { k1, k2, t1, t2 } = await firstSession();
      const before = await bucketOnDisk();

      const reloaded = newClient();
      // Let the new client's own start-up reads finish before holding the listing.
      await settle(50);
      const release = holdAttributionLoad();
      const map = reloaded.getORMap<string, string>(MAP);
      await settle(50);
      // The records are in, the attribution is not.
      expect(map.get('K')).toEqual(['live']);
      expect(map.getSnapshot().keyTombstones.size).toBe(0);

      const commitWrite = jest.spyOn(storage, 'commitWrite');
      const [t3] = map.remove('K', 'live');
      await settle();

      // The remove itself is durable at once; it does not wait for the load.
      expect(commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op))).toHaveLength(1);
      // But the bucket is not written from a map that has not read it yet.
      expect(await bucketOnDisk()).toEqual(before);

      release();
      await settle(50);

      const expected = new Map([
        [k1, [t1]],
        [k2, [t2]],
        ['K', [t3]],
      ]);
      expect(await bucketOnDisk()).toEqual(expected);
      for (const [key, tags] of expected) {
        expect(sorted(map.getKeyTombstones(key))).toEqual(tags);
      }
    });

    test('a load whose attribution cannot be listed leaves the bucket as it was, and does not keep later writes held', async () => {
      await firstSession();
      const before = await bucketOnDisk();

      const reloaded = newClient();
      await settle(50);
      let fail: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => {
        fail = resolve;
      });
      const listing = jest.spyOn(storage, 'getAllMetaKeys').mockImplementation(async () => {
        await gate;
        throw new Error('simulated listing failure');
      });
      jest.spyOn(logger, 'error').mockImplementation(() => undefined);
      const map = reloaded.getORMap<string, string>(MAP);
      await settle(50);
      map.add('K', 'second');
      await settle();

      map.remove('K', 'live');
      await settle();
      fail();
      await settle(50);
      listing.mockRestore();

      // The map never received what the bucket holds, so the bucket the remove
      // left out of its commit is not written from it.
      expect(map.getSnapshot().keyTombstones.size).toBe(1);
      expect(await bucketOnDisk()).toEqual(before);

      // The hold ended with the load: the next remove commits its bucket.
      const commitWrite = jest.spyOn(storage, 'commitWrite');
      map.remove('K', 'second');
      await settle();
      const removeCommits = commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op));
      expect(removeCommits).toHaveLength(1);
      expect(removeCommits[0][0].map((m) => m.key)).toContain(bucketKeyOf('K'));
    });

    test.each([
      [
        'TopGunClient.restoreORMap',
        (reloaded: TopGunClient) => {
          reloaded.getORMap<string, string>(MAP);
        },
      ],
      [
        'SyncEngine.instantiateAndRestoreOrMap',
        (reloaded: TopGunClient) => {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the second restore seam is private to the engine and only reachable from a sync start
          void (reloaded as any).syncEngine.instantiateAndRestoreOrMap(MAP);
        },
      ],
    ])(
      'a server-reported remove before the load through %s finishes leaves all three keys attributed',
      async (_name, startLoad) => {
        const { k1, k2, t1, t2, liveTag } = await firstSession();
        const before = await bucketOnDisk();

        const reloaded = newClient();
        await settle(50);
        const release = holdAttributionLoad();
        startLoad(reloaded);
        await settle(50);
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test delivers a server event straight to the engine
        const engine = (reloaded as any).syncEngine;
        const map = engine.maps.get(MAP) as ORMap<string, string>;
        expect(map.get('K')).toEqual(['live']);
        expect(map.getSnapshot().keyTombstones.size).toBe(0);

        // Not awaited here: the event's persist may wait for the load to finish.
        const applied = engine.applyServerEvent(
          MAP,
          'OR_REMOVE',
          'K',
          undefined,
          undefined,
          liveTag,
        );
        await settle();
        expect(map.get('K')).toEqual([]);
        expect(await bucketOnDisk()).toEqual(before);

        release();
        await applied;
        await settle(50);

        const expected = new Map([
          [k1, [t1]],
          [k2, [t2]],
          ['K', [liveTag]],
        ]);
        expect(await bucketOnDisk()).toEqual(expected);
        for (const [key, tags] of expected) {
          expect(sorted(map.getKeyTombstones(key))).toEqual(tags);
        }
      },
    );
  });

  describe('server-reported attribution that changes no record', () => {
    test('a replace and an absence reset are both written to storage', async () => {
      const client = newClient();
      const map = client.getORMap<string, string>(MAP);
      map.add('K', 'live');
      await settle();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test injects server responses straight into the engine's sync handler
      const handler = (client as any).syncEngine.orMapSyncHandler;
      const recordsBefore = await storage.get(`${MAP}:K`);

      // The server attributes a tag to K and sends back the record K already holds.
      await handler.handleORMapSyncRespLeaf({
        mapName: MAP,
        entries: [{ key: 'K', records: map.getRecords('K'), tombstones: ['server-tag'] }],
      });
      expect(await storage.getMeta(bucketKeyOf('K'))).toEqual([['K', ['server-tag']]]);
      expect(await attributionMetaKeys()).toEqual([bucketKeyOf('K')]);

      // The server reports an empty map: the attribution falls back to the
      // pending local removes, of which there are none.
      await handler.handleORMapSyncRespRoot({ mapName: MAP, rootHash: 0 });
      expect(await storage.getMeta(bucketKeyOf('K'))).toEqual([]);
      expect(await attributionMetaKeys()).toEqual([bucketKeyOf('K')]);

      expect(map.get('K')).toEqual(['live']);
      expect(await storage.get(`${MAP}:K`)).toEqual(recordsBefore);
    });
  });

  describe('held-map enumeration', () => {
    const heldNames = async (c: TopGunClient): Promise<string[]> =>
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the held-set computation is private to the engine
      sorted(await (c as any).syncEngine.computeHeldOrMapNames());

    test('attribution keys alone do not announce a map', async () => {
      await storage.setMeta('__sys__:ormapBackfillDone', true);
      await storage.setMeta(bucketKeyOf('K'), [['K', ['t']]]);
      await storage.setMeta(`${BUCKET_PREFIX}ab`, []);
      await storage.setMeta(UNBUCKETED_META, [['K', ['t']]]);

      expect(await heldNames(newClient())).toEqual([]);
    });

    test('buckets of a map whose name ends like a reserved key yield no map of their own', async () => {
      // Such names cannot be opened (a map name may not contain a colon), but a
      // store written before that rule can hold them. Only the keys that really
      // are a marker or a tombstone set may announce a name, and then the
      // map's real one.
      await storage.setMeta('__sys__:ormapBackfillDone', true);
      for (const name of ['x:keyTombstones', 'a:tombstones', 'b:ormap']) {
        for (const bucket of ['00', 'ab', 'ff']) {
          await storage.setMeta(orMapKeyTombstonesBucketKey(name, bucket), [['K', ['t']]]);
        }
      }
      expect(await heldNames(newClient())).toEqual([]);

      await storage.setMeta('__sys__:x:keyTombstones:ormap', 1);
      await storage.setMeta('__sys__:a:tombstones:tombstones', []);
      expect(await heldNames(newClient())).toEqual(['a:tombstones', 'x:keyTombstones']);
    });

    test('a map with a marker, tombstones and attribution is enumerated once, under its own name', async () => {
      const map = newClient().getORMap<string, string>(MAP);
      map.add('K', 'v');
      await settle();
      map.remove('K', 'v');
      await settle();
      expect(sorted(await storage.getAllMetaKeys())).toEqual(
        sorted([`__sys__:${MAP}:ormap`, TOMBSTONES_META, bucketKeyOf('K')]),
      );

      // A fresh process that has not opened the map discovers it from storage.
      expect(await heldNames(newClient())).toEqual([MAP]);
    });
  });
});
