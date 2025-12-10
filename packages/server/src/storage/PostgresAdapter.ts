import { Pool, PoolConfig } from 'pg';
import { LWWRecord } from '@topgunbuild/core';
import { IServerStorage, StorageValue } from './IServerStorage';

export interface PostgresAdapterOptions {
  tableName?: string;
}

const DEFAULT_TABLE_NAME = 'topgun_maps';
const TABLE_NAME_REGEX = /^[a-zA-Z_][a-zA-Z0-9_]*$/;

function validateTableName(name: string): void {
  if (!TABLE_NAME_REGEX.test(name)) {
    throw new Error(
      `Invalid table name "${name}". Table name must start with a letter or underscore and contain only alphanumeric characters and underscores.`
    );
  }
}

export class PostgresAdapter implements IServerStorage {
  private pool: Pool;
  private tableName: string;

  constructor(configOrPool: PoolConfig | Pool, options?: PostgresAdapterOptions) {
    if (configOrPool instanceof Pool || (configOrPool as any).connect) {
      this.pool = configOrPool as Pool;
    } else {
      this.pool = new Pool(configOrPool as PoolConfig);
    }

    const tableName = options?.tableName ?? DEFAULT_TABLE_NAME;
    validateTableName(tableName);
    this.tableName = tableName;
  }

  async initialize(): Promise<void> {
    const client = await this.pool.connect();
    try {
      // Create a generic table for storing key-value pairs per map
      // schema: map_name (text), key (text), value (jsonb), timestamp_millis (bigint), timestamp_counter (int), node_id (text), is_deleted (boolean)
      await client.query(`
        CREATE TABLE IF NOT EXISTS ${this.tableName} (
          map_name TEXT NOT NULL,
          key TEXT NOT NULL,
          value JSONB,
          ts_millis BIGINT NOT NULL,
          ts_counter INTEGER NOT NULL,
          ts_node_id TEXT NOT NULL,
          is_deleted BOOLEAN DEFAULT FALSE,
          PRIMARY KEY (map_name, key)
        );
      `);
    } finally {
      client.release();
    }
  }

  async close(): Promise<void> {
    await this.pool.end();
  }

  async load(mapName: string, key: string): Promise<StorageValue<any> | undefined> {
    const res = await this.pool.query(
      `SELECT value, ts_millis, ts_counter, ts_node_id, is_deleted 
       FROM ${this.tableName} 
       WHERE map_name = $1 AND key = $2`,
      [mapName, key]
    );

    if (res.rows.length === 0) return undefined;

    const row = res.rows[0];
    return this.mapRowToRecord(row);
  }

  async loadAll(mapName: string, keys: string[]): Promise<Map<string, StorageValue<any>>> {
    const result = new Map<string, StorageValue<any>>();
    if (keys.length === 0) return result;

    const res = await this.pool.query(
      `SELECT key, value, ts_millis, ts_counter, ts_node_id, is_deleted 
       FROM ${this.tableName} 
       WHERE map_name = $1 AND key = ANY($2)`,
      [mapName, keys]
    );

    for (const row of res.rows) {
      result.set(row.key, this.mapRowToRecord(row));
    }

    return result;
  }

  async loadAllKeys(mapName: string): Promise<string[]> {
    const res = await this.pool.query(
      `SELECT key FROM ${this.tableName} WHERE map_name = $1`,
      [mapName]
    );
    return res.rows.map(row => row.key);
  }

  async store(mapName: string, key: string, record: StorageValue<any>): Promise<void> {
    let value: any;
    let tsMillis: number;
    let tsCounter: number;
    let tsNodeId: string;
    let isDeleted: boolean;

    if (this.isORMapValue(record)) {
        // Store ORMap data
        // We use a special marker in ts_node_id to distinguish ORMap data from LWW data
        value = record;
        tsMillis = 0;
        tsCounter = 0;
        tsNodeId = '__ORMAP__';
        isDeleted = false;
    } else {
        // LWWRecord
        const lww = record as LWWRecord<any>;
        value = lww.value;
        tsMillis = lww.timestamp.millis;
        tsCounter = lww.timestamp.counter;
        tsNodeId = lww.timestamp.nodeId;
        isDeleted = lww.value === null;
    }

    await this.pool.query(
      `INSERT INTO ${this.tableName} (map_name, key, value, ts_millis, ts_counter, ts_node_id, is_deleted)
       VALUES ($1, $2, $3, $4, $5, $6, $7)
       ON CONFLICT (map_name, key) DO UPDATE SET
         value = EXCLUDED.value,
         ts_millis = EXCLUDED.ts_millis,
         ts_counter = EXCLUDED.ts_counter,
         ts_node_id = EXCLUDED.ts_node_id,
         is_deleted = EXCLUDED.is_deleted`,
      [
        mapName,
        key,
        JSON.stringify(value),
        tsMillis,
        tsCounter,
        tsNodeId,
        isDeleted
      ]
    );
  }

  async storeAll(mapName: string, records: Map<string, StorageValue<any>>): Promise<void> {
    const client = await this.pool.connect();
    try {
      await client.query('BEGIN');
      // Note: For high performance, this should use UNNEST or multi-row INSERT.
      // Keeping loop for simplicity in MVP alignment.
      for (const [key, record] of records) {
        await this.store(mapName, key, record); 
      }
      await client.query('COMMIT');
    } catch (e) {
      await client.query('ROLLBACK');
      throw e;
    } finally {
      client.release();
    }
  }

  async delete(mapName: string, key: string): Promise<void> {
    await this.pool.query(`DELETE FROM ${this.tableName} WHERE map_name = $1 AND key = $2`, [mapName, key]);
  }

  async deleteAll(mapName: string, keys: string[]): Promise<void> {
    if (keys.length === 0) return;
    await this.pool.query(
      `DELETE FROM ${this.tableName} WHERE map_name = $1 AND key = ANY($2)`, 
      [mapName, keys]
    );
  }

  private mapRowToRecord(row: any): StorageValue<any> {
    if (row.ts_node_id === '__ORMAP__') {
        // It's an ORMap value (ORMapValue or ORMapTombstones)
        return row.value as StorageValue<any>;
    }

    // It's LWWRecord
    return {
      value: row.is_deleted ? null : row.value,
      timestamp: {
        millis: Number(row.ts_millis),
        counter: row.ts_counter,
        nodeId: row.ts_node_id
      }
    };
  }

  private isORMapValue(record: any): boolean {
      return (record && typeof record === 'object' && (record.type === 'OR' || record.type === 'OR_TOMBSTONES'));
  }
}
