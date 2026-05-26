import type { LWWRecord, ORMapRecord } from '@topgunbuild/core';

export interface OpLogEntry {
  id?: number; // Auto-increment
  key: string;
  op: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE';
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op log value type varies by map; the adapter stores it opaquely
  value?: any; // For LWW PUT
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- LWWRecord value type erased in the op log; maps use their own generic at runtime
  record?: LWWRecord<any>; // LWW Record
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMapRecord value type erased in the op log
  orRecord?: ORMapRecord<any>; // ORMap Record
  orTag?: string; // ORMap Remove Tag
  hlc?: string; // Serialized timestamp
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- raw timestamp object shape varies (may be Timestamp or HLC object); stored opaquely in the op log
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- return includes `any` to cover metadata keys that are not LWWRecord or ORMapRecord; callers narrow the result
  get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- storage adapter is value-schema agnostic; accepts any serialisable value from the caller
  put(key: string, value: any): Promise<void>;
  remove(key: string): Promise<void>;

  // Metadata / Special keys
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- meta values have no fixed schema; callers store arbitrary primitives (strings, numbers, booleans) under meta keys
  getMeta(key: string): Promise<any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- meta values have no fixed schema; callers store arbitrary primitives under meta keys
  setMeta(key: string, value: any): Promise<void>;

  // Batch
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- batch put accepts a mixed-value map; values are stored by IDB structured clone regardless of shape
  batchPut(entries: Map<string, any>): Promise<void>;

  // OpLog
  appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number>;
  getPendingOps(): Promise<OpLogEntry[]>;
  markOpsSynced(lastId: number): Promise<void>;

  // Iteration
  getAllKeys(): Promise<string[]>;
}
