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

/**
 * A single store-level mutation that participates in an atomic {@link IStorageAdapter.commitWrite}.
 * `put` writes `value` under `key`; `remove` deletes `key`. `store` selects the durable
 * namespace: `'kv'` for record/value entries, `'meta'` for ORMap tombstone lists and
 * sync watermarks.
 */
export interface StorageMutation {
  store: 'kv' | 'meta';
  type: 'put' | 'remove';
  key: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- mutation values are storage-schema agnostic (LWWRecord, ORMapRecord[], tombstone arrays); stored opaquely
  value?: any;
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
  /**
   * Compacts the op log by **deleting** every op with `id <= lastId`. A synced op has no
   * further use — the durable KV/meta record written alongside it (see {@link commitWrite})
   * is the source of truth — so retaining acked ops only bloats the store unboundedly.
   * (Previously this flipped a `synced` flag and never deleted.)
   */
  markOpsSynced(lastId: number): Promise<void>;

  /**
   * Deletes a single op by its numeric auto-increment id. Used by the `drop-oldest`
   * backpressure strategy to keep memory and disk in agreement (otherwise a dropped op
   * resurrects on the next reload). Callers holding a stringified id must coerce to a
   * number at the boundary.
   */
  deleteOp(id: number): Promise<void>;

  /**
   * Atomically commits a set of KV/meta mutations together with the op-log entry that
   * records the mutation, in a **single** durable transaction, returning the appended op's
   * auto-increment id. This is the crash-consistent local-write primitive: a record and its
   * pending op are never observable independently (no record-without-op, no op-without-record).
   *
   * Adapters that cannot span stores in one transaction MUST still implement the method as
   * an awaited sequential best-effort commit (op appended last so a partial failure leaves a
   * record without an op rather than the reverse).
   */
  commitWrite(mutations: StorageMutation[], op: Omit<OpLogEntry, 'id'>): Promise<number>;

  // Iteration
  getAllKeys(): Promise<string[]>;
}
