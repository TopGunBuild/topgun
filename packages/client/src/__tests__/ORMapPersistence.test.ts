import { TopGunClient } from '../TopGunClient';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { LWWRecord, ORMapRecord } from '@topgunbuild/core';

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
