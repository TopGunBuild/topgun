import { newDb } from 'pg-mem';
import { PostgresAdapter } from '../storage/PostgresAdapter';
import { LWWRecord, ORMapRecord } from '@topgunbuild/core';
import { ORMapValue } from '../storage/IServerStorage';

describe('PostgresAdapter (Integration via pg-mem)', () => {
  let adapter: PostgresAdapter;
  let db: any;
  let pool: any;

  beforeEach(async () => {
    db = newDb();
    const { Pool } = db.adapters.createPg();
    pool = new Pool();
    adapter = new PostgresAdapter(pool);
    await adapter.initialize();
  });

  afterEach(async () => {
    await adapter.close();
  });

  test('should initialize and create table', async () => {
    // Check if table exists
    const res = await pool.query(`
      SELECT table_name
      FROM information_schema.tables
      WHERE table_schema = 'public'
      AND table_name = 'topgun_maps'
    `);
    expect(res.rows.length).toBe(1);
  });

  test('should use custom table name when provided', async () => {
    const customDb = newDb();
    const { Pool: CustomPool } = customDb.adapters.createPg();
    const customPool = new CustomPool();
    const customAdapter = new PostgresAdapter(customPool, { tableName: 'my_custom_table' });
    await customAdapter.initialize();

    const res = await customPool.query(`
      SELECT table_name
      FROM information_schema.tables
      WHERE table_schema = 'public'
      AND table_name = 'my_custom_table'
    `);
    expect(res.rows.length).toBe(1);

    // Verify data is stored in custom table
    const record: LWWRecord<string> = {
      value: 'test',
      timestamp: { millis: 100, counter: 1, nodeId: 'A' }
    };
    await customAdapter.store('map1', 'key1', record);

    const loaded = await customAdapter.load('map1', 'key1');
    expect(loaded).toEqual(record);

    await customAdapter.close();
  });

  test('should reject invalid table names', () => {
    expect(() => new PostgresAdapter(pool, { tableName: 'invalid-name' }))
      .toThrow('Invalid table name');
    expect(() => new PostgresAdapter(pool, { tableName: '123table' }))
      .toThrow('Invalid table name');
    expect(() => new PostgresAdapter(pool, { tableName: 'table; DROP TABLE users;' }))
      .toThrow('Invalid table name');
    expect(() => new PostgresAdapter(pool, { tableName: '' }))
      .toThrow('Invalid table name');
  });

  test('should accept valid table names', () => {
    expect(() => new PostgresAdapter(pool, { tableName: 'valid_table_name' })).not.toThrow();
    expect(() => new PostgresAdapter(pool, { tableName: '_underscore_start' })).not.toThrow();
    expect(() => new PostgresAdapter(pool, { tableName: 'Table123' })).not.toThrow();
  });

  test('should store and load LWWRecord (primitive)', async () => {
    const record: LWWRecord<string> = {
      value: 'hello world',
      timestamp: {
        millis: 1000,
        counter: 1,
        nodeId: 'node-1'
      }
    };

    await adapter.store('map1', 'key1', record);
    const loaded = await adapter.load('map1', 'key1');

    expect(loaded).toEqual(record);
  });

  test('should store and load LWWRecord (object)', async () => {
    const record: LWWRecord<any> = {
      value: { foo: 'bar', num: 123 },
      timestamp: {
        millis: 2000,
        counter: 2,
        nodeId: 'node-2'
      }
    };

    await adapter.store('map1', 'key2', record);
    const loaded = await adapter.load('map1', 'key2');

    expect(loaded).toEqual(record);
  });

  test('should store and load ORMap', async () => {
    const orMapValue: ORMapValue<string> = {
      type: 'OR',
      records: [
        {
          value: 'item1',
          timestamp: { millis: 100, counter: 1, nodeId: 'A' },
          tag: 'tag1'
        },
        {
          value: 'item2',
          timestamp: { millis: 200, counter: 2, nodeId: 'B' },
          tag: 'tag2'
        }
      ]
    };

    await adapter.store('map1', 'ormap1', orMapValue);
    const loaded = await adapter.load('map1', 'ormap1');

    expect(loaded).toEqual(orMapValue);
    
    // Verify raw storage hack
    const res = await pool.query("SELECT ts_node_id, value FROM topgun_maps WHERE key = 'ormap1'");
    expect(res.rows[0].ts_node_id).toBe('__ORMAP__');
    expect(res.rows[0].value).toEqual(orMapValue);
  });

  test('should handle loadAll', async () => {
    const record1: LWWRecord<string> = {
      value: 'v1',
      timestamp: { millis: 100, counter: 0, nodeId: 'A' }
    };
    const record2: LWWRecord<string> = {
      value: 'v2',
      timestamp: { millis: 100, counter: 0, nodeId: 'A' }
    };

    await adapter.store('map1', 'k1', record1);
    await adapter.store('map1', 'k2', record2);

    const result = await adapter.loadAll('map1', ['k1', 'k2', 'k3']);
    
    expect(result.size).toBe(2);
    expect(result.get('k1')).toEqual(record1);
    expect(result.get('k2')).toEqual(record2);
    expect(result.has('k3')).toBe(false);
  });

  test('should handle deleteAll', async () => {
    const record: LWWRecord<string> = {
      value: 'v1',
      timestamp: { millis: 100, counter: 0, nodeId: 'A' }
    };

    await adapter.store('map1', 'k1', record);
    await adapter.store('map1', 'k2', record);

    await adapter.deleteAll('map1', ['k1']);
    
    const r1 = await adapter.load('map1', 'k1');
    const r2 = await adapter.load('map1', 'k2');

    expect(r1).toBeUndefined();
    expect(r2).toBeDefined();
  });

  test('should update existing record (UPSERT)', async () => {
    const recordV1: LWWRecord<string> = {
      value: 'v1',
      timestamp: { millis: 100, counter: 1, nodeId: 'A' }
    };
    const recordV2: LWWRecord<string> = {
      value: 'v2',
      timestamp: { millis: 200, counter: 2, nodeId: 'A' }
    };

    await adapter.store('map1', 'key1', recordV1);
    let loaded = await adapter.load('map1', 'key1');
    expect(loaded).toEqual(recordV1);

    await adapter.store('map1', 'key1', recordV2);
    loaded = await adapter.load('map1', 'key1');
    expect(loaded).toEqual(recordV2);
  });

  test('loadAllKeys should return all keys for a map', async () => {
    const record: LWWRecord<string> = {
      value: 'v',
      timestamp: { millis: 0, counter: 0, nodeId: 'A' }
    };
    await adapter.store('map1', 'k1', record);
    await adapter.store('map1', 'k2', record);
    await adapter.store('map2', 'k3', record);

    const keys = await adapter.loadAllKeys('map1');
    expect(keys.sort()).toEqual(['k1', 'k2'].sort());
  });

  test('should loadAll with mixed LWW and ORMap records', async () => {
    const lwwRec: LWWRecord<string> = {
      value: 'lww',
      timestamp: { millis: 1, counter: 0, nodeId: 'A' }
    };
    const orRec: ORMapValue<string> = {
      type: 'OR',
      records: []
    };

    await adapter.store('mixed', 'lww', lwwRec);
    await adapter.store('mixed', 'or', orRec);

    const result = await adapter.loadAll('mixed', ['lww', 'or']);
    
    expect(result.size).toBe(2);
    expect(result.get('lww')).toEqual(lwwRec);
    expect(result.get('or')).toEqual(orRec);
  });
});

