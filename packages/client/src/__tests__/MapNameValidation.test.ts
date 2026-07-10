import { TopGunClient } from '../TopGunClient';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { LWWRecord, ORMapRecord } from '@topgunbuild/core';

// Minimal in-memory storage adapter (mirrors ORMapPersistence.test.ts) — these
// tests exercise the local map-creation guard + restore seams, no network.
class MemoryStorageAdapter implements IStorageAdapter {
  kvStore: Map<string, any> = new Map();
  metaStore: Map<string, any> = new Map();
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
    for (const [key, value] of entries) this.kvStore.set(key, value);
  }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opLog.length + 1;
    const newEntry = { ...entry, id, synced: 0 } as OpLogEntry;
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

// Mock WebSocket — local-only tests never dial out; keep Jest's worker from being
// held open by the real undici connection-timeout (see ORMapPersistence.test.ts).
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
    queueMicrotask(() => {
      if (this.onopen) this.onopen();
    });
  }
};

afterAll(() => {
  (globalThis as any).WebSocket = originalWebSocket;
});

describe('Map-name validation guard (getMap/getORMap)', () => {
  let storage: MemoryStorageAdapter;
  let client: TopGunClient;
  const extraClients: TopGunClient[] = [];

  beforeEach(() => {
    storage = new MemoryStorageAdapter();
    client = new TopGunClient({ serverUrl: 'ws://localhost:1234', storage });
  });

  afterEach(async () => {
    await client.close();
    for (const c of extraClients) await c.close();
    extraClients.length = 0;
  });

  // AC1
  describe('AC1 — colon in a map NAME is rejected, colon-free names succeed', () => {
    it('getMap("a:b") throws an Error identifying ":"', () => {
      expect(() => client.getMap('a:b')).toThrow(/":"/);
    });

    it('getORMap("a:b") throws an Error identifying ":"', () => {
      expect(() => client.getORMap('a:b')).toThrow(/":"/);
    });

    it('getMap("a-b") and getORMap("a-b") succeed unchanged', () => {
      expect(() => client.getMap('a-b')).not.toThrow();
      expect(() => client.getORMap('or-a-b')).not.toThrow();
    });
  });

  // AC2 — guard runs BEFORE registration/restore
  describe('AC2 — a rejected name leaves this.maps and the sync registry unmodified', () => {
    it('getMap rejection creates no partial map in either registry', () => {
      const clientMaps: Map<string, unknown> = (client as any).maps;
      const syncMaps: Map<string, unknown> = (client as any).syncEngine.maps;
      const beforeClient = clientMaps.size;
      const beforeSync = syncMaps.size;

      expect(() => client.getMap('bad:name')).toThrow();

      expect(clientMaps.has('bad:name')).toBe(false);
      expect(syncMaps.has('bad:name')).toBe(false);
      expect(clientMaps.size).toBe(beforeClient);
      expect(syncMaps.size).toBe(beforeSync);
    });

    it('getORMap rejection creates no partial map in either registry', () => {
      const clientMaps: Map<string, unknown> = (client as any).maps;
      const syncMaps: Map<string, unknown> = (client as any).syncEngine.maps;

      expect(() => client.getORMap('bad:name')).toThrow();

      expect(clientMaps.has('bad:name')).toBe(false);
      expect(syncMaps.has('bad:name')).toBe(false);
    });
  });

  // AC3 — colon in a KEY is still allowed and round-trips through persistence
  describe('AC3 — colon in a map KEY is allowed and round-trips', () => {
    it('getORMap("tags").add("post:123", "x") persists and restores', async () => {
      const map = client.getORMap<string, string>('tags');
      map.add('post:123', 'x');
      await new Promise((r) => setTimeout(r, 10));

      // The composite key persists under the injective storage key "tags:post:123".
      const stored = await storage.get('tags:post:123');
      expect(Array.isArray(stored)).toBe(true);
      expect((stored as any[]).map((rec) => rec.value)).toContain('x');

      // Round-trip: a fresh client over the SAME storage restores the key intact.
      const reopened = new TopGunClient({ serverUrl: 'ws://localhost:1234', storage });
      extraClients.push(reopened);
      const restored = reopened.getORMap<string, string>('tags');
      await new Promise((r) => setTimeout(r, 50));
      expect(restored.get('post:123')).toEqual(['x']);
    });
  });

  // AC3c — empty-string name rejected, registry unmodified
  describe('AC3c — empty-string name is rejected', () => {
    it('getMap("") and getORMap("") throw', () => {
      expect(() => client.getMap('')).toThrow(/empty/);
      expect(() => client.getORMap('')).toThrow(/empty/);
    });

    it('the rejection leaves this.maps and the sync registry unmodified', () => {
      const clientMaps: Map<string, unknown> = (client as any).maps;
      const syncMaps: Map<string, unknown> = (client as any).syncEngine.maps;
      const beforeClient = clientMaps.size;
      const beforeSync = syncMaps.size;

      expect(() => client.getMap('')).toThrow();

      expect(clientMaps.has('')).toBe(false);
      expect(syncMaps.has('')).toBe(false);
      expect(clientMaps.size).toBe(beforeClient);
      expect(syncMaps.size).toBe(beforeSync);
    });
  });

  // AC5b — longest-held-name restore-guard in the TopGunClient primary restore path
  describe('AC5b — TopGunClient restore path skips a key owned by a LONGER held name', () => {
    it('restoring map "a" does NOT absorb "a:b:k" when both "a" and "a:b" are held', async () => {
      // Seed a legacy store: sibling "a" holds key "k1"; a colon-named "a:b" holds
      // key "k" (storage key "a:b:k"). Both are surfaced in the connection held-set.
      const ts = { millis: Date.now(), counter: 0, nodeId: 'A' };
      storage.kvStore.set('a:k1', [{ value: 'own', timestamp: ts, tag: 't-own' }]);
      storage.kvStore.set('a:b:k', [{ value: 'sibling', timestamp: ts, tag: 't-sib' }]);

      // Drive the shared discriminator: both names held for this connection.
      (client as any).syncEngine.heldOrMapNames = new Set(['a', 'a:b']);

      const mapA = client.getORMap<string, string>('a');
      await new Promise((r) => setTimeout(r, 50));

      // "a" restores its own key but NOT the longer-name key "b:k".
      expect(mapA.get('k1')).toEqual(['own']);
      expect(mapA.get('b:k')).toEqual([]);
    });
  });
});
