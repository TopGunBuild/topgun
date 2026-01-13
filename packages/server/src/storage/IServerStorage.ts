import { LWWRecord, ORMapRecord } from '@topgunbuild/core';

/**
 * Marker used in ts_node_id column to distinguish ORMap values from LWW records.
 * This allows storing both CRDT types in the same table structure.
 * Used by PostgresAdapter and BetterSqlite3Adapter.
 */
export const ORMAP_MARKER = '__ORMAP__';

export type ORMapValue<V> = {
  type: 'OR';
  records: ORMapRecord<V>[];
};

export type ORMapTombstones = {
  type: 'OR_TOMBSTONES';
  tags: string[];
};

export type StorageValue<V> = LWWRecord<V> | ORMapValue<V> | ORMapTombstones;

/**
 * Server Persistence Interface (MapStore).
 * Aligned with specifications/06_SERVER_INTEGRATIONS.md
 * 
 * Note: We include mapName in all methods because a single storage adapter
 * instance typically handles multiple maps in the system (e.g. single DB connection).
 */
export interface IServerStorage {
  /**
   * Initialize the storage connection.
   */
  initialize(): Promise<void>;

  /**
   * Close the storage connection.
   */
  close(): Promise<void>;

  /**
   * Loads the value of a given key.
   * Called when a client requests a key that is not in the Server RAM (if using partial loading),
   * or during sync.
   */
  load(mapName: string, key: string): Promise<StorageValue<any> | undefined>;

  /**
   * Loads multiple keys.
   * Optimization for batch requests.
   */
  loadAll(mapName: string, keys: string[]): Promise<Map<string, StorageValue<any>>>;

  /**
   * Loads all keys from the store for a specific map.
   * Used for pre-loading the cache on startup or understanding dataset size.
   */
  loadAllKeys(mapName: string): Promise<string[]>;

  /**
   * Stores the key-value pair.
   */
  store(mapName: string, key: string, record: StorageValue<any>): Promise<void>;

  /**
   * Stores multiple entries.
   * Used for efficient batch writes to the DB.
   */
  storeAll(mapName: string, records: Map<string, StorageValue<any>>): Promise<void>;

  /**
   * Deletes the entry with the given key.
   */
  delete(mapName: string, key: string): Promise<void>;

  /**
   * Deletes multiple entries.
   */
  deleteAll(mapName: string, keys: string[]): Promise<void>;
}
