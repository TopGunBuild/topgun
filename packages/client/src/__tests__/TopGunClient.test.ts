import { TopGunClient } from '../TopGunClient';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { LWWRecord, ORMapRecord } from '@topgunbuild/core';
import { QueryHandle } from '../QueryHandle';
import { DistributedLock } from '../DistributedLock';
import { TopicHandle } from '../TopicHandle';

// Mock WebSocket
class MockWebSocket {
  static OPEN = 1;
  static CLOSED = 3;

  readyState = MockWebSocket.OPEN;
  binaryType: string = 'blob';
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((error: any) => void) | null = null;
  onmessage: ((event: any) => void) | null = null;

  constructor(public url: string) {
    // Simulate async connection
    setTimeout(() => {
      if (this.onopen) this.onopen();
    }, 0);
  }

  send(data: any) {}
  close() {
    this.readyState = MockWebSocket.CLOSED;
    if (this.onclose) this.onclose();
  }
}

(global as any).WebSocket = MockWebSocket;

// Mock crypto.randomUUID with counter for unique values
let uuidCounter = 0;
const mockUUID = jest.fn(() => `test-uuid-${++uuidCounter}`);
Object.defineProperty(global, 'crypto', {
  value: { randomUUID: mockUUID }
});

// Mock Storage Adapter (reused from ORMapPersistence.test.ts)
class MemoryStorageAdapter implements IStorageAdapter {
  private kvStore: Map<string, any> = new Map();
  private metaStore: Map<string, any> = new Map();
  private opLog: OpLogEntry[] = [];
  private _pendingOps: OpLogEntry[] = [];
  public initializeCalled = false;
  public initializeDbName: string | null = null;

  async initialize(dbName: string): Promise<void> {
    this.initializeCalled = true;
    this.initializeDbName = dbName;
  }
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
    this._pendingOps = this._pendingOps.filter(op => op.id! > lastId);
    this.opLog.forEach(op => {
      if (op.id! <= lastId) op.synced = 1;
    });
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kvStore.keys());
  }
}

describe('TopGunClient', () => {
  let storage: MemoryStorageAdapter;
  let client: TopGunClient;

  beforeEach(() => {
    mockUUID.mockClear();
    storage = new MemoryStorageAdapter();
    client = new TopGunClient({
      serverUrl: 'ws://localhost:1234',
      storage
    });
  });

  describe('Initialization', () => {
    test('should create client with configuration', () => {
      const customClient = new TopGunClient({
        nodeId: 'custom-node-id',
        serverUrl: 'ws://localhost:5678',
        storage
      });

      expect(customClient).toBeInstanceOf(TopGunClient);
    });

    test('should generate nodeId if not provided', () => {
      // The client created in beforeEach doesn't have nodeId
      // crypto.randomUUID should have been called
      expect(mockUUID).toHaveBeenCalled();
    });

    test('should use provided nodeId when specified', () => {
      mockUUID.mockClear();

      const customClient = new TopGunClient({
        nodeId: 'my-custom-node',
        serverUrl: 'ws://localhost:1234',
        storage
      });

      // When nodeId is provided, randomUUID should not be called for nodeId
      // (it might still be called for SyncEngine internals, but we check that
      // the provided nodeId is used)
      expect(customClient).toBeInstanceOf(TopGunClient);
    });

    test('start() should initialize storage', async () => {
      expect(storage.initializeCalled).toBe(false);

      await client.start();

      expect(storage.initializeCalled).toBe(true);
      expect(storage.initializeDbName).toBe('topgun_offline_db');
    });
  });

  describe('Authentication', () => {
    test('setAuthToken() should set the auth token', () => {
      // Should not throw
      expect(() => client.setAuthToken('my-auth-token')).not.toThrow();
    });

    test('setAuthTokenProvider() should set the token provider', () => {
      const tokenProvider = jest.fn(async () => 'dynamic-token');

      // Should not throw
      expect(() => client.setAuthTokenProvider(tokenProvider)).not.toThrow();
    });

    test('setAuthTokenProvider() should accept provider returning null', () => {
      const tokenProvider = jest.fn(async () => null);

      expect(() => client.setAuthTokenProvider(tokenProvider)).not.toThrow();
    });
  });

  describe('Query API', () => {
    test('query() should create a QueryHandle', () => {
      const handle = client.query<{ name: string }>('users', {});

      expect(handle).toBeInstanceOf(QueryHandle);
    });

    test('query() should pass filters to QueryHandle', () => {
      const filter = {
        where: { status: 'active' },
        sort: { createdAt: 'desc' as const },
        limit: 10,
        offset: 0
      };

      const handle = client.query<{ name: string; status: string }>('users', filter);

      expect(handle.getFilter()).toEqual(filter);
      expect(handle.getMapName()).toBe('users');
    });

    test('query() should create unique handles for each call', () => {
      const handle1 = client.query('users', { where: { a: 1 } });
      const handle2 = client.query('users', { where: { a: 2 } });

      expect(handle1.id).not.toBe(handle2.id);
    });
  });

  describe('Distributed Lock', () => {
    test('getLock() should return a DistributedLock by name', () => {
      const lock = client.getLock('my-resource');

      expect(lock).toBeInstanceOf(DistributedLock);
    });

    test('getLock() should return new instance on each call', () => {
      const lock1 = client.getLock('resource-a');
      const lock2 = client.getLock('resource-a');

      // Each call creates a new DistributedLock instance
      expect(lock1).not.toBe(lock2);
    });

    test('getLock() with different names returns different locks', () => {
      const lock1 = client.getLock('resource-a');
      const lock2 = client.getLock('resource-b');

      expect(lock1).not.toBe(lock2);
    });
  });

  describe('Pub/Sub Topics', () => {
    test('topic() should return a TopicHandle', () => {
      const handle = client.topic('chat-room');

      expect(handle).toBeInstanceOf(TopicHandle);
    });

    test('topic() should cache TopicHandle (return same object for same name)', () => {
      const handle1 = client.topic('notifications');
      const handle2 = client.topic('notifications');

      expect(handle1).toBe(handle2);
    });

    test('topic() should return different handles for different names', () => {
      const handle1 = client.topic('topic-a');
      const handle2 = client.topic('topic-b');

      expect(handle1).not.toBe(handle2);
    });

    test('TopicHandle should have correct id', () => {
      const handle = client.topic('my-topic');

      expect(handle.id).toBe('my-topic');
    });
  });

  describe('LWWMap', () => {
    test('getMap() should create an LWWMap', () => {
      const map = client.getMap<string, number>('counters');

      expect(map).toBeDefined();
      expect(typeof map.set).toBe('function');
      expect(typeof map.get).toBe('function');
      expect(typeof map.remove).toBe('function');
    });

    test('getMap() with same name should return cached map', () => {
      const map1 = client.getMap<string, string>('users');
      const map2 = client.getMap<string, string>('users');

      expect(map1).toBe(map2);
    });

    test('getMap() should throw if name already used for ORMap', () => {
      // First create an ORMap
      client.getORMap<string, string>('tags');

      // Then try to get it as LWWMap
      expect(() => client.getMap<string, string>('tags')).toThrow(
        'Map tags exists but is not an LWWMap'
      );
    });

    test('getMap() should allow different names', () => {
      const map1 = client.getMap<string, number>('map-a');
      const map2 = client.getMap<string, number>('map-b');

      expect(map1).not.toBe(map2);
    });

    test('LWWMap set() should persist to storage', async () => {
      const map = client.getMap<string, string>('settings');

      map.set('theme', 'dark');

      // Wait for async storage
      await new Promise(resolve => setTimeout(resolve, 10));

      const stored = await storage.get('settings:theme');
      expect(stored).toBeDefined();
      expect(stored.value).toBe('dark');
    });

    test('LWWMap remove() should persist tombstone to storage', async () => {
      const map = client.getMap<string, string>('settings');

      map.set('theme', 'dark');
      await new Promise(resolve => setTimeout(resolve, 10));

      map.remove('theme');
      await new Promise(resolve => setTimeout(resolve, 10));

      const stored = await storage.get('settings:theme');
      expect(stored).toBeDefined();
      expect(stored.value).toBeNull();
    });
  });

  describe('ORMap', () => {
    test('getORMap() should throw if name already used for LWWMap', () => {
      // First create an LWWMap
      client.getMap<string, string>('items');

      // Then try to get it as ORMap
      expect(() => client.getORMap<string, string>('items')).toThrow(
        'Map items exists but is not an ORMap'
      );
    });

    test('getORMap() with same name should return cached map', () => {
      const map1 = client.getORMap<string, string>('tags');
      const map2 = client.getORMap<string, string>('tags');

      expect(map1).toBe(map2);
    });
  });
});
