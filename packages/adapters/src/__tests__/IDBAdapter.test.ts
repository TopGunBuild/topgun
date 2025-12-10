import { IDBAdapter } from '../IDBAdapter';
import { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';

describe('IDBAdapter', () => {
  let adapter: IDBAdapter;
  let dbCounter = 0;

  // Use unique database name for each test to ensure isolation
  const getUniqueDbName = () => `test_db_${Date.now()}_${dbCounter++}`;

  beforeEach(async () => {
    adapter = new IDBAdapter();
  });

  afterEach(async () => {
    await adapter.close();
  });

  describe('initialization', () => {
    it('should initialize database successfully', async () => {
      const dbName = getUniqueDbName();
      await expect(adapter.initialize(dbName)).resolves.not.toThrow();
    });

    it('should allow multiple initializations with different names', async () => {
      const adapter1 = new IDBAdapter();
      const adapter2 = new IDBAdapter();

      await adapter1.initialize(getUniqueDbName());
      await adapter2.initialize(getUniqueDbName());

      await adapter1.close();
      await adapter2.close();
    });
  });

  describe('put and get operations', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should store and retrieve a simple value', async () => {
      await adapter.put('key1', { name: 'test', value: 123 });
      const result = await adapter.get('key1');
      expect(result).toEqual({ name: 'test', value: 123 });
    });

    it('should store and retrieve a string value', async () => {
      await adapter.put('stringKey', 'hello world');
      const result = await adapter.get('stringKey');
      expect(result).toBe('hello world');
    });

    it('should store and retrieve a number value', async () => {
      await adapter.put('numberKey', 42);
      const result = await adapter.get('numberKey');
      expect(result).toBe(42);
    });

    it('should store and retrieve a boolean value', async () => {
      await adapter.put('boolKey', true);
      const result = await adapter.get('boolKey');
      expect(result).toBe(true);
    });

    it('should store and retrieve an array value', async () => {
      const arr = [1, 2, 3, { nested: true }];
      await adapter.put('arrayKey', arr);
      const result = await adapter.get('arrayKey');
      expect(result).toEqual(arr);
    });

    it('should store and retrieve a nested object', async () => {
      const nested = {
        level1: {
          level2: {
            level3: {
              value: 'deep'
            }
          }
        }
      };
      await adapter.put('nestedKey', nested);
      const result = await adapter.get('nestedKey');
      expect(result).toEqual(nested);
    });

    it('should return undefined for non-existent key', async () => {
      const result = await adapter.get('nonExistentKey');
      expect(result).toBeUndefined();
    });

    it('should overwrite existing value with same key', async () => {
      await adapter.put('key1', 'original');
      await adapter.put('key1', 'updated');
      const result = await adapter.get('key1');
      expect(result).toBe('updated');
    });

    it('should store null value', async () => {
      await adapter.put('nullKey', null);
      const result = await adapter.get('nullKey');
      expect(result).toBeNull();
    });
  });

  describe('remove operation', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should remove an existing key', async () => {
      await adapter.put('keyToRemove', 'value');
      expect(await adapter.get('keyToRemove')).toBe('value');

      await adapter.remove('keyToRemove');
      expect(await adapter.get('keyToRemove')).toBeUndefined();
    });

    it('should not throw when removing non-existent key', async () => {
      await expect(adapter.remove('nonExistentKey')).resolves.not.toThrow();
    });

    it('should only remove the specified key', async () => {
      await adapter.put('key1', 'value1');
      await adapter.put('key2', 'value2');
      await adapter.put('key3', 'value3');

      await adapter.remove('key2');

      expect(await adapter.get('key1')).toBe('value1');
      expect(await adapter.get('key2')).toBeUndefined();
      expect(await adapter.get('key3')).toBe('value3');
    });
  });

  describe('meta operations', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should store and retrieve metadata', async () => {
      await adapter.setMeta('lastSync', 1234567890);
      const result = await adapter.getMeta('lastSync');
      expect(result).toBe(1234567890);
    });

    it('should return undefined for non-existent meta key', async () => {
      const result = await adapter.getMeta('nonExistentMeta');
      expect(result).toBeUndefined();
    });

    it('should keep meta store separate from kv store', async () => {
      await adapter.put('sharedName', 'kv_value');
      await adapter.setMeta('sharedName', 'meta_value');

      expect(await adapter.get('sharedName')).toBe('kv_value');
      expect(await adapter.getMeta('sharedName')).toBe('meta_value');
    });

    it('should overwrite existing meta value', async () => {
      await adapter.setMeta('config', { version: 1 });
      await adapter.setMeta('config', { version: 2 });
      const result = await adapter.getMeta('config');
      expect(result).toEqual({ version: 2 });
    });
  });

  describe('batchPut operation', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should store multiple entries in a batch', async () => {
      const entries = new Map<string, any>([
        ['batch1', { data: 'first' }],
        ['batch2', { data: 'second' }],
        ['batch3', { data: 'third' }]
      ]);

      await adapter.batchPut(entries);

      expect(await adapter.get('batch1')).toEqual({ data: 'first' });
      expect(await adapter.get('batch2')).toEqual({ data: 'second' });
      expect(await adapter.get('batch3')).toEqual({ data: 'third' });
    });

    it('should handle empty batch', async () => {
      const entries = new Map<string, any>();
      await expect(adapter.batchPut(entries)).resolves.not.toThrow();
    });

    it('should overwrite existing keys in batch', async () => {
      await adapter.put('existing', 'old');

      const entries = new Map<string, any>([
        ['existing', 'new'],
        ['fresh', 'value']
      ]);

      await adapter.batchPut(entries);

      expect(await adapter.get('existing')).toBe('new');
      expect(await adapter.get('fresh')).toBe('value');
    });
  });

  describe('getAllKeys operation', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should return empty array when no keys exist', async () => {
      const keys = await adapter.getAllKeys();
      expect(keys).toEqual([]);
    });

    it('should return all stored keys', async () => {
      await adapter.put('key1', 'value1');
      await adapter.put('key2', 'value2');
      await adapter.put('key3', 'value3');

      const keys = await adapter.getAllKeys();
      expect(keys.sort()).toEqual(['key1', 'key2', 'key3']);
    });

    it('should not include removed keys', async () => {
      await adapter.put('key1', 'value1');
      await adapter.put('key2', 'value2');
      await adapter.remove('key1');

      const keys = await adapter.getAllKeys();
      expect(keys).toEqual(['key2']);
    });

    it('should not include meta keys', async () => {
      await adapter.put('kvKey', 'kvValue');
      await adapter.setMeta('metaKey', 'metaValue');

      const keys = await adapter.getAllKeys();
      expect(keys).toEqual(['kvKey']);
    });
  });

  describe('opLog operations', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should append operation to log and return id', async () => {
      const entry: Omit<OpLogEntry, 'id'> = {
        key: 'users/123',
        op: 'PUT',
        value: { name: 'Alice' },
        synced: 0,
        mapName: 'users'
      };

      const id = await adapter.appendOpLog(entry);
      expect(typeof id).toBe('number');
      expect(id).toBeGreaterThan(0);
    });

    it('should return incremented ids for sequential appends', async () => {
      const entry1: Omit<OpLogEntry, 'id'> = {
        key: 'key1',
        op: 'PUT',
        value: 'value1',
        synced: 0,
        mapName: 'test'
      };
      const entry2: Omit<OpLogEntry, 'id'> = {
        key: 'key2',
        op: 'PUT',
        value: 'value2',
        synced: 0,
        mapName: 'test'
      };

      const id1 = await adapter.appendOpLog(entry1);
      const id2 = await adapter.appendOpLog(entry2);

      expect(id2).toBeGreaterThan(id1);
    });

    it('should get pending operations (synced=0)', async () => {
      await adapter.appendOpLog({
        key: 'key1',
        op: 'PUT',
        value: 'value1',
        synced: 0,
        mapName: 'test'
      });
      await adapter.appendOpLog({
        key: 'key2',
        op: 'REMOVE',
        synced: 0,
        mapName: 'test'
      });

      const pending = await adapter.getPendingOps();
      expect(pending.length).toBe(2);
      expect(pending.every(op => op.synced === 0)).toBe(true);
    });

    it('should mark operations as synced', async () => {
      const id1 = await adapter.appendOpLog({
        key: 'key1',
        op: 'PUT',
        value: 'value1',
        synced: 0,
        mapName: 'test'
      });
      const id2 = await adapter.appendOpLog({
        key: 'key2',
        op: 'PUT',
        value: 'value2',
        synced: 0,
        mapName: 'test'
      });
      const id3 = await adapter.appendOpLog({
        key: 'key3',
        op: 'PUT',
        value: 'value3',
        synced: 0,
        mapName: 'test'
      });

      // Mark first two as synced
      await adapter.markOpsSynced(id2);

      const pending = await adapter.getPendingOps();
      expect(pending.length).toBe(1);
      expect(pending[0].key).toBe('key3');
    });

    it('should handle different operation types', async () => {
      await adapter.appendOpLog({
        key: 'lww/123',
        op: 'PUT',
        record: { value: 'test', hlc: '123' },
        synced: 0,
        mapName: 'lwwMap'
      });

      await adapter.appendOpLog({
        key: 'or/456',
        op: 'OR_ADD',
        orRecord: { value: 'item', tag: 'abc', hlc: '456' },
        synced: 0,
        mapName: 'orMap'
      });

      await adapter.appendOpLog({
        key: 'or/456',
        op: 'OR_REMOVE',
        orTag: 'abc',
        synced: 0,
        mapName: 'orMap'
      });

      const pending = await adapter.getPendingOps();
      expect(pending.length).toBe(3);
      expect(pending.map(p => p.op)).toEqual(['PUT', 'OR_ADD', 'OR_REMOVE']);
    });
  });

  describe('persistence across operations', () => {
    it('should persist data between get/put cycles', async () => {
      const dbName = getUniqueDbName();
      await adapter.initialize(dbName);

      // Store data
      await adapter.put('persistent', { preserved: true });
      await adapter.setMeta('metaPersist', 123);

      // Data should still be there after multiple reads
      for (let i = 0; i < 5; i++) {
        expect(await adapter.get('persistent')).toEqual({ preserved: true });
        expect(await adapter.getMeta('metaPersist')).toBe(123);
      }
    });
  });

  describe('large data volumes', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should handle many entries', async () => {
      const count = 100;
      const entries = new Map<string, any>();

      for (let i = 0; i < count; i++) {
        entries.set(`key_${i}`, { index: i, data: `value_${i}` });
      }

      await adapter.batchPut(entries);

      const keys = await adapter.getAllKeys();
      expect(keys.length).toBe(count);

      // Verify random samples
      expect(await adapter.get('key_0')).toEqual({ index: 0, data: 'value_0' });
      expect(await adapter.get('key_50')).toEqual({ index: 50, data: 'value_50' });
      expect(await adapter.get('key_99')).toEqual({ index: 99, data: 'value_99' });
    });

    it('should handle large values', async () => {
      const largeArray = Array(1000).fill(null).map((_, i) => ({
        id: i,
        name: `Item ${i}`,
        description: 'Lorem ipsum dolor sit amet '.repeat(10)
      }));

      await adapter.put('largeValue', largeArray);
      const result = await adapter.get('largeValue');

      expect(result.length).toBe(1000);
      expect(result[0].id).toBe(0);
      expect(result[999].id).toBe(999);
    });

    it('should handle many pending operations', async () => {
      const count = 50;

      for (let i = 0; i < count; i++) {
        await adapter.appendOpLog({
          key: `op_${i}`,
          op: 'PUT',
          value: { index: i },
          synced: 0,
          mapName: 'test'
        });
      }

      const pending = await adapter.getPendingOps();
      expect(pending.length).toBe(count);
    });
  });

  describe('error handling', () => {
    it('should handle operations before initialization gracefully', async () => {
      // Operations before initialize should not crash (though they may return undefined)
      const uninitializedAdapter = new IDBAdapter();
      const result = await uninitializedAdapter.get('anyKey');
      expect(result).toBeUndefined();
    });
  });

  describe('LWWRecord and ORMapRecord types', () => {
    beforeEach(async () => {
      await adapter.initialize(getUniqueDbName());
    });

    it('should store and retrieve LWWRecord-like structures', async () => {
      const lwwRecord = {
        value: { name: 'Test User', email: 'test@example.com' },
        hlc: '1234567890-0-node1'
      };

      await adapter.put('users/user1', lwwRecord);
      const result = await adapter.get<typeof lwwRecord.value>('users/user1');

      expect(result).toEqual(lwwRecord);
    });

    it('should store and retrieve ORMapRecord-like structures', async () => {
      const orMapRecords = [
        { value: 'item1', tag: 'tag1', hlc: '1234567890-0-node1' },
        { value: 'item2', tag: 'tag2', hlc: '1234567891-0-node1' },
        { value: 'item3', tag: 'tag3', hlc: '1234567892-0-node2' }
      ];

      await adapter.put('set/items', orMapRecords);
      const result = await adapter.get('set/items');

      expect(result).toEqual(orMapRecords);
      expect(result.length).toBe(3);
    });
  });
});
