import Database from 'better-sqlite3';
import { LWWRecord } from '@topgunbuild/core';
import { IServerStorage, StorageValue } from './IServerStorage';

/**
 * SQLite Development Adapter
 *
 * Zero-dependency development mode using better-sqlite3.
 * Allows developers to run TopGun WITHOUT Docker for quick prototyping.
 *
 * LIMITATIONS:
 * - No clustering support (SQLite = single-node only)
 * - Lower performance than PostgreSQL (sufficient for dev)
 * - File locking on Windows (use WSL for multi-process)
 * - Max ~500K records (sufficient for development)
 *
 * RECOMMENDED FOR:
 * - Local development
 * - Quick prototyping
 * - CI/CD testing
 * - Small single-node deployments
 *
 * NOT RECOMMENDED FOR:
 * - Production clusters
 * - High-write workloads
 * - Multi-tenant deployments
 */

export interface BetterSqlite3Config {
  /**
   * Path to the SQLite database file.
   * Use ':memory:' for in-memory database (lost on restart).
   */
  filename: string;

  /**
   * Enable verbose logging of SQL statements.
   */
  verbose?: boolean;

  /**
   * Custom table name (default: 'topgun_maps')
   */
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

export class BetterSqlite3Adapter implements IServerStorage {
  private db: Database.Database;
  private tableName: string;
  private statements!: {
    load: Database.Statement;
    loadAll: Database.Statement;
    loadAllKeys: Database.Statement;
    store: Database.Statement;
    delete: Database.Statement;
    deleteAll: Database.Statement;
  };

  constructor(config: BetterSqlite3Config) {
    const tableName = config.tableName ?? DEFAULT_TABLE_NAME;
    validateTableName(tableName);
    this.tableName = tableName;

    this.db = new Database(config.filename, {
      verbose: config.verbose ? console.log : undefined,
    });

    // Enable WAL mode for better concurrent read performance
    this.db.pragma('journal_mode = WAL');
    // Synchronous mode: NORMAL is a good balance of safety and speed
    this.db.pragma('synchronous = NORMAL');
    // Increase cache size for better performance
    this.db.pragma('cache_size = -64000'); // 64MB
  }

  async initialize(): Promise<void> {
    // Create table
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS ${this.tableName} (
        map_name TEXT NOT NULL,
        key TEXT NOT NULL,
        value TEXT,
        ts_millis INTEGER NOT NULL,
        ts_counter INTEGER NOT NULL,
        ts_node_id TEXT NOT NULL,
        is_deleted INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (strftime('%s', 'now') * 1000),
        updated_at INTEGER DEFAULT (strftime('%s', 'now') * 1000),
        PRIMARY KEY (map_name, key)
      );

      CREATE INDEX IF NOT EXISTS idx_${this.tableName}_map ON ${this.tableName}(map_name);
      CREATE INDEX IF NOT EXISTS idx_${this.tableName}_updated ON ${this.tableName}(updated_at);
    `);

    // Prepare statements for better performance
    this.prepareStatements();

    console.log(`[BetterSqlite3Adapter] Initialized with table: ${this.tableName}`);
  }

  private prepareStatements(): void {
    this.statements = {
      load: this.db.prepare(`
        SELECT value, ts_millis, ts_counter, ts_node_id, is_deleted
        FROM ${this.tableName}
        WHERE map_name = ? AND key = ?
      `),

      loadAll: this.db.prepare(`
        SELECT key, value, ts_millis, ts_counter, ts_node_id, is_deleted
        FROM ${this.tableName}
        WHERE map_name = ? AND key IN (SELECT value FROM json_each(?))
      `),

      loadAllKeys: this.db.prepare(`
        SELECT key FROM ${this.tableName} WHERE map_name = ?
      `),

      store: this.db.prepare(`
        INSERT INTO ${this.tableName} (map_name, key, value, ts_millis, ts_counter, ts_node_id, is_deleted, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, strftime('%s', 'now') * 1000)
        ON CONFLICT(map_name, key) DO UPDATE SET
          value = excluded.value,
          ts_millis = excluded.ts_millis,
          ts_counter = excluded.ts_counter,
          ts_node_id = excluded.ts_node_id,
          is_deleted = excluded.is_deleted,
          updated_at = strftime('%s', 'now') * 1000
      `),

      delete: this.db.prepare(`
        DELETE FROM ${this.tableName} WHERE map_name = ? AND key = ?
      `),

      deleteAll: this.db.prepare(`
        DELETE FROM ${this.tableName} WHERE map_name = ? AND key IN (SELECT value FROM json_each(?))
      `),
    };
  }

  async close(): Promise<void> {
    this.db.close();
    console.log('[BetterSqlite3Adapter] Database closed');
  }

  async load(mapName: string, key: string): Promise<StorageValue<any> | undefined> {
    const row = this.statements.load.get(mapName, key) as any;

    if (!row) return undefined;

    return this.mapRowToRecord(row);
  }

  async loadAll(mapName: string, keys: string[]): Promise<Map<string, StorageValue<any>>> {
    const result = new Map<string, StorageValue<any>>();
    if (keys.length === 0) return result;

    const rows = this.statements.loadAll.all(mapName, JSON.stringify(keys)) as any[];

    for (const row of rows) {
      result.set(row.key, this.mapRowToRecord(row));
    }

    return result;
  }

  async loadAllKeys(mapName: string): Promise<string[]> {
    const rows = this.statements.loadAllKeys.all(mapName) as any[];
    return rows.map((row) => row.key);
  }

  async store(mapName: string, key: string, record: StorageValue<any>): Promise<void> {
    let value: string;
    let tsMillis: number;
    let tsCounter: number;
    let tsNodeId: string;
    let isDeleted: number;

    if (this.isORMapValue(record)) {
      // Store ORMap data
      // Use special marker in ts_node_id to distinguish from LWW data
      value = JSON.stringify(record);
      tsMillis = 0;
      tsCounter = 0;
      tsNodeId = '__ORMAP__';
      isDeleted = 0;
    } else {
      // LWWRecord
      const lww = record as LWWRecord<any>;
      value = JSON.stringify(lww.value);
      tsMillis = lww.timestamp.millis;
      tsCounter = lww.timestamp.counter;
      tsNodeId = lww.timestamp.nodeId;
      isDeleted = lww.value === null ? 1 : 0;
    }

    this.statements.store.run(
      mapName,
      key,
      value,
      tsMillis,
      tsCounter,
      tsNodeId,
      isDeleted
    );
  }

  async storeAll(mapName: string, records: Map<string, StorageValue<any>>): Promise<void> {
    const storeMany = this.db.transaction((recs: Map<string, StorageValue<any>>) => {
      for (const [key, record] of recs) {
        let value: string;
        let tsMillis: number;
        let tsCounter: number;
        let tsNodeId: string;
        let isDeleted: number;

        if (this.isORMapValue(record)) {
          value = JSON.stringify(record);
          tsMillis = 0;
          tsCounter = 0;
          tsNodeId = '__ORMAP__';
          isDeleted = 0;
        } else {
          const lww = record as LWWRecord<any>;
          value = JSON.stringify(lww.value);
          tsMillis = lww.timestamp.millis;
          tsCounter = lww.timestamp.counter;
          tsNodeId = lww.timestamp.nodeId;
          isDeleted = lww.value === null ? 1 : 0;
        }

        this.statements.store.run(
          mapName,
          key,
          value,
          tsMillis,
          tsCounter,
          tsNodeId,
          isDeleted
        );
      }
    });

    storeMany(records);
  }

  async delete(mapName: string, key: string): Promise<void> {
    this.statements.delete.run(mapName, key);
  }

  async deleteAll(mapName: string, keys: string[]): Promise<void> {
    if (keys.length === 0) return;
    this.statements.deleteAll.run(mapName, JSON.stringify(keys));
  }

  private mapRowToRecord(row: any): StorageValue<any> {
    if (row.ts_node_id === '__ORMAP__') {
      // It's an ORMap value (ORMapValue or ORMapTombstones)
      return JSON.parse(row.value) as StorageValue<any>;
    }

    // It's LWWRecord
    return {
      value: row.is_deleted ? null : JSON.parse(row.value),
      timestamp: {
        millis: row.ts_millis,
        counter: row.ts_counter,
        nodeId: row.ts_node_id,
      },
    };
  }

  private isORMapValue(record: any): boolean {
    return record && typeof record === 'object' && (record.type === 'OR' || record.type === 'OR_TOMBSTONES');
  }

  // Additional utility methods

  /**
   * Health check - verify database is accessible
   */
  async ping(): Promise<boolean> {
    try {
      this.db.prepare('SELECT 1').get();
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Get database statistics
   */
  getStats(): { pageCount: number; pageSize: number; totalSize: number } {
    const pageCount = (this.db.pragma('page_count') as any)[0].page_count;
    const pageSize = (this.db.pragma('page_size') as any)[0].page_size;
    return {
      pageCount,
      pageSize,
      totalSize: pageCount * pageSize,
    };
  }

  /**
   * Vacuum database to reclaim space
   */
  vacuum(): void {
    this.db.exec('VACUUM');
  }

  /**
   * Run checkpoint to flush WAL to main database file
   */
  checkpoint(): void {
    this.db.pragma('wal_checkpoint(TRUNCATE)');
  }
}
