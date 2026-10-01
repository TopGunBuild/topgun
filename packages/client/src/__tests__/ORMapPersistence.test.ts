import { TopGunClient } from '../TopGunClient';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { HLC, LWWRecord, ORMap, ORMapRecord } from '@topgunbuild/core';
import { restoreOrMapKeyTombstones } from '../utils/orMapKeyTombstones';
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
  const KEY_TOMBSTONES_META = `__sys__:${MAP}:keyTombstones`;

  let storage: MemoryStorageAdapter;
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
    test('commits the attribution in the same batch as the key records and the map-wide tombstones', async () => {
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
        key: KEY_TOMBSTONES_META,
        value: [['list1', [tag]]],
      });
    });

    test('commits the whole current attribution, including every tag of a multi-tag remove, with its first op', async () => {
      const map = newClient().getORMap<string, string>(MAP);
      map.add('other', 'x');
      map.add('list1', 'dup');
      map.add('list1', 'dup');
      await settle();
      const [otherTag] = map.remove('other', 'x');
      await settle();

      const commitWrite = jest.spyOn(storage, 'commitWrite');
      const tags = map.remove('list1', 'dup');
      expect(tags).toHaveLength(2);
      await settle();

      // Two OR_REMOVE ops, one atomic commit: the second tag is an op-only append.
      const removeCommits = commitWrite.mock.calls.filter(([, op]) => isRemoveOp(op));
      expect(removeCommits).toHaveLength(1);
      const attribution = removeCommits[0][0].find((m) => m.key === KEY_TOMBSTONES_META);
      const persisted = new Map(attribution?.value as Array<[string, string[]]>);
      expect(sorted(persisted.keys())).toEqual(['list1', 'other']);
      expect(persisted.get('other')).toEqual([otherTag]);
      expect(sorted(persisted.get('list1') ?? [])).toEqual(sorted(tags));
    });
  });

  describe.each(restorePaths)('reload through %s', (_name, reload) => {
    test('restores the attribution, so the Merkle root equals the root before the reload', async () => {
      const map = newClient().getORMap<string, string>(MAP);
      map.add('gone', 'x');
      map.add('mixed', 'keep');
      map.add('mixed', 'drop');
      await settle();
      // `gone` ends up holding tombstones only; `mixed` holds a live record and
      // an attributed tombstone. Both leaves depend on the attribution.
      const [goneTag] = map.remove('gone', 'x');
      const [dropTag] = map.remove('mixed', 'drop');
      await settle();

      const rootBefore = map.getMerkleTree().getRootHash();
      expect(rootBefore).not.toBe(0);
      expect(map.get('gone')).toEqual([]);

      const restored = await reload(newClient());

      expect(restored.getMerkleTree().getRootHash()).toBe(rootBefore);
      expect(sorted(restored.getKeyTombstones('gone'))).toEqual([goneTag]);
      expect(sorted(restored.getKeyTombstones('mixed'))).toEqual([dropTag]);
      expect(restored.get('mixed')).toEqual(['keep']);
    });

    test('a key named __proto__ round-trips', async () => {
      const prototypeKeysBefore = Object.getOwnPropertyNames(Object.prototype).sort();
      const map = newClient().getORMap<string, string>(MAP);
      map.add('__proto__', 'v');
      await settle();
      const [tag] = map.remove('__proto__', 'v');
      await settle();

      expect(await storage.getMeta(KEY_TOMBSTONES_META)).toEqual([['__proto__', [tag]]]);
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
      expect(await storage.getMeta(KEY_TOMBSTONES_META)).toBeUndefined();
    });

    test('a persisted record whose tag is attributed but missing from the map-wide tombstones stays removed', async () => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      await storage.put(`${MAP}:elsewhere`, [{ value: 'stale', timestamp, tag: 'moved_tag' }]);
      await storage.setMeta(TOMBSTONES_META, []);
      await storage.setMeta(KEY_TOMBSTONES_META, [['list', ['moved_tag']]]);

      const restored = await reload(newClient());

      expect(restored.get('elsewhere')).toEqual([]);
      expect(restored.getTombstones()).toContain('moved_tag');
      expect(sorted(restored.getKeyTombstones('list'))).toEqual(['moved_tag']);
    });

    test.each([
      ['a non-array entry', 'garbage'],
      ['an object keyed by map key', { list: ['t1'] }],
      ['null', null],
    ])('%s is ignored and the map still loads', async (_shape, persisted) => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      await storage.put(`${MAP}:list`, [{ value: 'live', timestamp, tag: 'live_tag' }]);
      await storage.setMeta(KEY_TOMBSTONES_META, persisted);

      const restored = await reload(newClient());

      expect(restored.get('list')).toEqual(['live']);
      expect(restored.getSnapshot().keyTombstones.size).toBe(0);
    });

    test('malformed pairs are skipped one by one; the readable ones are restored', async () => {
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'A' };
      await storage.put(`${MAP}:list`, [{ value: 'live', timestamp, tag: 'live_tag' }]);
      await storage.setMeta(KEY_TOMBSTONES_META, [
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
        'not-a-pair',
        null,
        ['only-a-key'],
        [42, ['numeric-key']],
        ['tags-not-an-array', 'oops'],
        ['empty', []],
        ['good', ['t1', 7, 't2']],
      ]);

      expect(sorted(map.getKeyTombstones('good'))).toEqual(['t1', 't2']);
      expect(warn).toHaveBeenCalledTimes(1);
      expect(warn.mock.calls[0][0]).toEqual({ skipped: 6 });
    });

    test('a value that is not a list of pairs is reported', () => {
      const map = new ORMap<string, string>(new HLC('n1'));

      restoreOrMapKeyTombstones(map, { K: ['t1'] });

      expect(map.getSnapshot().keyTombstones.size).toBe(0);
      expect(warn).toHaveBeenCalledTimes(1);
    });

    test.each([
      ['an absent entry', undefined],
      ['an empty entry', []],
      ['a readable entry', [['K', ['t1']]]],
    ])('%s is not reported', (_shape, persisted) => {
      const map = new ORMap<string, string>(new HLC('n1'));

      restoreOrMapKeyTombstones(map, persisted);

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

      restoreOrMapKeyTombstones(map, [
        ['K', ['persisted']],
        ['other', ['elsewhere']],
      ]);

      expect(sorted(map.getKeyTombstones('K'))).toEqual(sorted([fresh, 'persisted']));
      expect(sorted(map.getKeyTombstones('other'))).toEqual(['elsewhere']);
      expect(sorted(map.getTombstones())).toEqual(sorted([fresh, 'persisted', 'elsewhere']));
    });
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
      expect(await storage.getMeta(KEY_TOMBSTONES_META)).toEqual([['K', ['server-tag']]]);

      // The server reports an empty map: the attribution falls back to the
      // pending local removes, of which there are none.
      await handler.handleORMapSyncRespRoot({ mapName: MAP, rootHash: 0 });
      expect(await storage.getMeta(KEY_TOMBSTONES_META)).toEqual([]);

      expect(map.get('K')).toEqual(['live']);
      expect(await storage.get(`${MAP}:K`)).toEqual(recordsBefore);
    });
  });

  describe('held-map enumeration', () => {
    const heldNames = async (c: TopGunClient): Promise<string[]> =>
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the held-set computation is private to the engine
      sorted(await (c as any).syncEngine.computeHeldOrMapNames());

    test('the attribution key alone does not announce a map', async () => {
      await storage.setMeta('__sys__:ormapBackfillDone', true);
      await storage.setMeta(KEY_TOMBSTONES_META, [['K', ['t']]]);

      expect(await heldNames(newClient())).toEqual([]);
    });

    test('a map with a marker, tombstones and attribution is enumerated once, under its own name', async () => {
      const map = newClient().getORMap<string, string>(MAP);
      map.add('K', 'v');
      await settle();
      map.remove('K', 'v');
      await settle();
      expect(sorted(await storage.getAllMetaKeys())).toEqual(
        sorted([`__sys__:${MAP}:ormap`, TOMBSTONES_META, KEY_TOMBSTONES_META]),
      );

      // A fresh process that has not opened the map discovers it from storage.
      expect(await heldNames(newClient())).toEqual([MAP]);
    });
  });
});
