import { BetterSqlite3Adapter } from '../../src/storage/BetterSqlite3Adapter';
import { StorageValue, ORMapValue, ORMapTombstones } from '../../src/storage/IServerStorage';
import fs from 'fs';
import path from 'path';

describe('BetterSqlite3Adapter', () => {
  let adapter: BetterSqlite3Adapter;
  const testDbPath = path.join(__dirname, 'test-topgun.db');

  beforeEach(async () => {
    // Clean up any existing test database
    if (fs.existsSync(testDbPath)) {
      fs.unlinkSync(testDbPath);
    }
    if (fs.existsSync(testDbPath + '-wal')) {
      fs.unlinkSync(testDbPath + '-wal');
    }
    if (fs.existsSync(testDbPath + '-shm')) {
      fs.unlinkSync(testDbPath + '-shm');
    }

    adapter = new BetterSqlite3Adapter({ filename: testDbPath });
    await adapter.initialize();
  });

  afterEach(async () => {
    await adapter.close();

    // Clean up test database
    if (fs.existsSync(testDbPath)) {
      fs.unlinkSync(testDbPath);
    }
    if (fs.existsSync(testDbPath + '-wal')) {
      fs.unlinkSync(testDbPath + '-wal');
    }
    if (fs.existsSync(testDbPath + '-shm')) {
      fs.unlinkSync(testDbPath + '-shm');
    }
  });

  describe('LWWRecord operations', () => {
    it('should store and load a LWWRecord', async () => {
      const mapName = 'test-map';
      const key = 'user-1';
      const record: StorageValue<any> = {
        value: { name: 'John', age: 30 },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      };

      await adapter.store(mapName, key, record);
      const result = await adapter.load(mapName, key);

      expect(result).not.toBeNull();
      expect((result as any).value).toEqual({ name: 'John', age: 30 });
      expect((result as any).timestamp.nodeId).toBe('node-1');
    });

    it('should return undefined for non-existent key', async () => {
      const result = await adapter.load('map', 'non-existent');
      expect(result).toBeUndefined();
    });

    it('should update existing record', async () => {
      const mapName = 'test-map';
      const key = 'user-1';

      const record1: StorageValue<any> = {
        value: { name: 'John' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      };

      const record2: StorageValue<any> = {
        value: { name: 'Jane' },
        timestamp: { millis: Date.now() + 1000, counter: 1, nodeId: 'node-1' },
      };

      await adapter.store(mapName, key, record1);
      await adapter.store(mapName, key, record2);

      const result = await adapter.load(mapName, key);
      expect((result as any).value).toEqual({ name: 'Jane' });
    });

    it('should handle deleted records (null value)', async () => {
      const mapName = 'test-map';
      const key = 'user-1';

      const record: StorageValue<any> = {
        value: null,
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      };

      await adapter.store(mapName, key, record);
      const result = await adapter.load(mapName, key);

      expect((result as any).value).toBeNull();
    });
  });

  describe('ORMap operations', () => {
    it('should store and load ORMapValue', async () => {
      const mapName = 'or-map';
      const key = 'item-1';

      const orMapValue: ORMapValue<any> = {
        type: 'OR',
        records: [
          {
            value: 'item-a',
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
            tag: 'tag-1',
          },
          {
            value: 'item-b',
            timestamp: { millis: Date.now(), counter: 1, nodeId: 'node-2' },
            tag: 'tag-2',
          },
        ],
      };

      await adapter.store(mapName, key, orMapValue);
      const result = await adapter.load(mapName, key) as ORMapValue<any>;

      expect(result.type).toBe('OR');
      expect(result.records).toHaveLength(2);
      expect(result.records[0].value).toBe('item-a');
      expect(result.records[1].value).toBe('item-b');
    });

    it('should store and load ORMapTombstones', async () => {
      const mapName = 'or-map';
      const key = '__tombstones__';

      const tombstones: ORMapTombstones = {
        type: 'OR_TOMBSTONES',
        tags: ['tag-1', 'tag-2', 'tag-3'],
      };

      await adapter.store(mapName, key, tombstones);
      const result = await adapter.load(mapName, key) as ORMapTombstones;

      expect(result.type).toBe('OR_TOMBSTONES');
      expect(result.tags).toEqual(['tag-1', 'tag-2', 'tag-3']);
    });
  });

  describe('Batch operations', () => {
    it('should load multiple keys at once', async () => {
      const mapName = 'batch-test';

      // Store multiple records
      await adapter.store(mapName, 'key-1', {
        value: 'value-1',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });
      await adapter.store(mapName, 'key-2', {
        value: 'value-2',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });
      await adapter.store(mapName, 'key-3', {
        value: 'value-3',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });

      const result = await adapter.loadAll(mapName, ['key-1', 'key-3']);

      expect(result.size).toBe(2);
      expect((result.get('key-1') as any).value).toBe('value-1');
      expect((result.get('key-3') as any).value).toBe('value-3');
    });

    it('should store multiple records in transaction', async () => {
      const mapName = 'batch-store';
      const records = new Map<string, StorageValue<any>>();

      for (let i = 0; i < 100; i++) {
        records.set(`key-${i}`, {
          value: `value-${i}`,
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node-1' },
        });
      }

      await adapter.storeAll(mapName, records);

      const keys = await adapter.loadAllKeys(mapName);
      expect(keys).toHaveLength(100);
    });

    it('should load all keys from a map', async () => {
      const mapName = 'keys-test';

      await adapter.store(mapName, 'alpha', {
        value: 'a',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });
      await adapter.store(mapName, 'beta', {
        value: 'b',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });

      const keys = await adapter.loadAllKeys(mapName);

      expect(keys).toHaveLength(2);
      expect(keys).toContain('alpha');
      expect(keys).toContain('beta');
    });
  });

  describe('Delete operations', () => {
    it('should delete a single key', async () => {
      const mapName = 'delete-test';

      await adapter.store(mapName, 'to-delete', {
        value: 'data',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });

      await adapter.delete(mapName, 'to-delete');

      const result = await adapter.load(mapName, 'to-delete');
      expect(result).toBeUndefined();
    });

    it('should delete multiple keys', async () => {
      const mapName = 'delete-many-test';

      await adapter.store(mapName, 'key-1', {
        value: '1',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });
      await adapter.store(mapName, 'key-2', {
        value: '2',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });
      await adapter.store(mapName, 'key-3', {
        value: '3',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });

      await adapter.deleteAll(mapName, ['key-1', 'key-3']);

      const keys = await adapter.loadAllKeys(mapName);
      expect(keys).toEqual(['key-2']);
    });
  });

  describe('Map isolation', () => {
    it('should isolate data between different maps', async () => {
      await adapter.store('map-a', 'key-1', {
        value: 'from-a',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });
      await adapter.store('map-b', 'key-1', {
        value: 'from-b',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });

      const fromA = await adapter.load('map-a', 'key-1');
      const fromB = await adapter.load('map-b', 'key-1');

      expect((fromA as any).value).toBe('from-a');
      expect((fromB as any).value).toBe('from-b');
    });
  });

  describe('Health check', () => {
    it('should return true when healthy', async () => {
      const result = await adapter.ping();
      expect(result).toBe(true);
    });
  });

  describe('Statistics', () => {
    it('should return database stats', async () => {
      const stats = adapter.getStats();

      expect(stats).toHaveProperty('pageCount');
      expect(stats).toHaveProperty('pageSize');
      expect(stats).toHaveProperty('totalSize');
      expect(typeof stats.totalSize).toBe('number');
    });
  });

  describe('In-memory mode', () => {
    let memAdapter: BetterSqlite3Adapter;

    beforeEach(async () => {
      memAdapter = new BetterSqlite3Adapter({ filename: ':memory:' });
      await memAdapter.initialize();
    });

    afterEach(async () => {
      await memAdapter.close();
    });

    it('should work with in-memory database', async () => {
      await memAdapter.store('mem-map', 'key', {
        value: 'memory-data',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' },
      });

      const result = await memAdapter.load('mem-map', 'key');
      expect((result as any).value).toBe('memory-data');
    });
  });
});
