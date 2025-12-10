import type { LWWRecord, ORMapRecord } from '@topgunbuild/core';

export interface OpLogEntry {
  id?: number; // Auto-increment
  key: string;
  op: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE';
  value?: any; // For LWW PUT
  record?: LWWRecord<any>; // LWW Record
  orRecord?: ORMapRecord<any>; // ORMap Record
  orTag?: string; // ORMap Remove Tag
  hlc?: string; // Serialized timestamp
  timestamp?: any; // Raw timestamp object if needed
  synced: number; // 0 or 1
  mapName: string; // Added mapName for filtering
}

export interface IStorageAdapter {
  initialize(dbName: string): Promise<void>;
  close(): Promise<void>;

  /**
   * Waits for the storage adapter to be fully initialized.
   * Optional - adapters that initialize synchronously may return immediately.
   */
  waitForReady?(): Promise<void>;

  // KV Operations
  get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined>;
  put(key: string, value: any): Promise<void>;
  remove(key: string): Promise<void>;

  // Metadata / Special keys
  getMeta(key: string): Promise<any>;
  setMeta(key: string, value: any): Promise<void>;

  // Batch
  batchPut(entries: Map<string, any>): Promise<void>;

  // OpLog
  appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number>;
  getPendingOps(): Promise<OpLogEntry[]>;
  markOpsSynced(lastId: number): Promise<void>;

  // Iteration
  getAllKeys(): Promise<string[]>;
}
