import type { LWWRecord, ORMapRecord } from '@topgunbuild/core';
import { IStorageAdapter, OpLogEntry, StorageMutation } from '@topgunbuild/client';
import { openDB } from 'idb';
import type { IDBPDatabase } from 'idb';

/**
 * Represents an operation queued before IndexedDB is ready.
 */
interface QueuedOperation {
  type:
    | 'put'
    | 'remove'
    | 'setMeta'
    | 'appendOpLog'
    | 'markOpsSynced'
    | 'batchPut'
    | 'commitWrite'
    | 'deleteOp';
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- queued operation args are heterogeneous across operation types; a discriminated union would require one args type per op type
  args: any[];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- resolve/reject carry the result of the eventual async operation whose type varies by op type
  resolve: (value: any) => void;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- error can be any thrown value from IDB transactions
  reject: (error: any) => void;
}

/**
 * Non-blocking IndexedDB adapter that allows immediate use before initialization completes.
 *
 * Operations are queued in memory and replayed once IndexedDB is ready.
 * This enables true "memory-first" behavior where the UI can render immediately
 * without waiting for IndexedDB to initialize (which can take 50-500ms).
 */
export class IDBAdapter implements IStorageAdapter {
  private dbPromise?: Promise<IDBPDatabase>;
  private db?: IDBPDatabase;
  private isReady = false;
  private operationQueue: QueuedOperation[] = [];
  private initPromise?: Promise<void>;

  /**
   * Initializes IndexedDB in the background.
   * Returns immediately - does NOT block on IndexedDB being ready.
   * Use waitForReady() if you need to ensure initialization is complete.
   */
  async initialize(dbName: string): Promise<void> {
    // Start initialization but don't await it — non-blocking by design
    this.initPromise = this.initializeInternal(dbName);
    // Return immediately
  }

  /**
   * Internal initialization that actually opens IndexedDB.
   */
  private async initializeInternal(dbName: string): Promise<void> {
    try {
      this.dbPromise = openDB(dbName, 2, {
        upgrade(db) {
          if (!db.objectStoreNames.contains('kv_store')) {
            db.createObjectStore('kv_store', { keyPath: 'key' });
          }
          if (!db.objectStoreNames.contains('op_log')) {
            db.createObjectStore('op_log', { keyPath: 'id', autoIncrement: true });
          }
          if (!db.objectStoreNames.contains('meta_store')) {
            db.createObjectStore('meta_store', { keyPath: 'key' });
          }
        },
      });

      this.db = await this.dbPromise;
      this.isReady = true;

      // Replay queued operations in the order they arrived
      await this.flushQueue();
    } catch (error) {
      // Re-throw to allow error handling
      throw error;
    }
  }

  /**
   * Waits for IndexedDB to be fully initialized.
   * Call this if you need guaranteed persistence before proceeding.
   */
  async waitForReady(): Promise<void> {
    if (this.isReady) return;
    if (this.initPromise) {
      await this.initPromise;
    }
  }

  /**
   * Flushes all queued operations once IndexedDB is ready.
   */
  private async flushQueue(): Promise<void> {
    const queue = this.operationQueue;
    this.operationQueue = [];

    for (const op of queue) {
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- result type varies across the switch cases; collecting into a common typed variable requires a union
        let result: any;
        switch (op.type) {
          case 'put':
            result = await this.putInternal(op.args[0], op.args[1]);
            break;
          case 'remove':
            result = await this.removeInternal(op.args[0]);
            break;
          case 'setMeta':
            result = await this.setMetaInternal(op.args[0], op.args[1]);
            break;
          case 'appendOpLog':
            result = await this.appendOpLogInternal(op.args[0]);
            break;
          case 'markOpsSynced':
            result = await this.markOpsSyncedInternal(op.args[0]);
            break;
          case 'batchPut':
            result = await this.batchPutInternal(op.args[0]);
            break;
          case 'commitWrite':
            result = await this.commitWriteInternal(op.args[0], op.args[1]);
            break;
          case 'deleteOp':
            result = await this.deleteOpInternal(op.args[0]);
            break;
        }
        op.resolve(result);
      } catch (error) {
        op.reject(error);
      }
    }
  }

  /**
   * Queues an operation if not ready, or executes immediately if ready.
   */
  private queueOrExecute<T>(
    type: QueuedOperation['type'],
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- args are forwarded to the queued operation and replayed; their types vary by operation
    args: any[],
    executor: () => Promise<T>,
  ): Promise<T> {
    if (this.isReady) {
      return executor();
    }

    return new Promise<T>((resolve, reject) => {
      this.operationQueue.push({ type, args, resolve, reject });
    });
  }

  async close(): Promise<void> {
    if (this.db) {
      this.db.close();
    }
  }

  // ============================================
  // Read Operations - Wait for ready
  // ============================================

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- return type includes `any` to cover metadata keys that are not LWWRecord or ORMapRecord (e.g. raw meta values)
  async get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined> {
    // Read operations must wait for DB to be ready
    await this.waitForReady();
    const result = await this.db?.get('kv_store', key);
    return result?.value;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- meta values have no fixed schema; callers store arbitrary primitives (strings, numbers, booleans) under meta keys
  async getMeta(key: string): Promise<any> {
    await this.waitForReady();
    const result = await this.db?.get('meta_store', key);
    return result?.value;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    await this.waitForReady();
    const all = await this.db?.getAll('op_log');
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IDB getAll returns untyped IDBValue; op_log entries are cast to OpLogEntry by the caller after this filter
    return all?.filter((op: any) => op.synced === 0) || [];
  }

  async getAllKeys(): Promise<string[]> {
    await this.waitForReady();
    return ((await this.db?.getAllKeys('kv_store')) as string[]) || [];
  }

  // ============================================
  // Write Operations - Queue if not ready
  // ============================================

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IStorageAdapter.put accepts any serialisable value; the adapter is storage-layer agnostic about value schema
  async put(key: string, value: any): Promise<void> {
    return this.queueOrExecute('put', [key, value], () => this.putInternal(key, value));
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- internal put mirrors the public signature; IDB structured clone accepts any serialisable value
  private async putInternal(key: string, value: any): Promise<void> {
    await this.db?.put('kv_store', { key, value });
  }

  async remove(key: string): Promise<void> {
    return this.queueOrExecute('remove', [key], () => this.removeInternal(key));
  }

  private async removeInternal(key: string): Promise<void> {
    await this.db?.delete('kv_store', key);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- meta values have no fixed schema; callers store arbitrary primitives (strings, numbers, booleans) under meta keys
  async setMeta(key: string, value: any): Promise<void> {
    return this.queueOrExecute('setMeta', [key, value], () => this.setMetaInternal(key, value));
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- internal setMeta mirrors the public signature; meta values are arbitrary primitives stored by the sync engine
  private async setMetaInternal(key: string, value: any): Promise<void> {
    await this.db?.put('meta_store', { key, value });
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- batch put accepts a mixed-value map; values are serialised by IDB structured clone regardless of shape
  async batchPut(entries: Map<string, any>): Promise<void> {
    return this.queueOrExecute('batchPut', [entries], () => this.batchPutInternal(entries));
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- internal batchPut mirrors the public signature; values are stored as-is by IDB structured clone
  private async batchPutInternal(entries: Map<string, any>): Promise<void> {
    const tx = this.db?.transaction('kv_store', 'readwrite');
    if (!tx) return;

    await Promise.all(
      Array.from(entries.entries()).map(([key, value]) => tx.store.put({ key, value })),
    );
    await tx.done;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op log entries arrive as OpLogEntry but IStorageAdapter.appendOpLog signature uses any to stay storage-backend agnostic
  async appendOpLog(entry: any): Promise<number> {
    return this.queueOrExecute('appendOpLog', [entry], () => this.appendOpLogInternal(entry));
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- internal appendOpLog mirrors public signature; IDB add returns the auto-incremented key
  private async appendOpLogInternal(entry: any): Promise<number> {
    const entryToSave = { ...entry, synced: 0 };
    return (await this.db?.add('op_log', entryToSave)) as number;
  }

  async markOpsSynced(lastId: number): Promise<void> {
    return this.queueOrExecute('markOpsSynced', [lastId], () => this.markOpsSyncedInternal(lastId));
  }

  // Compaction: DELETE acked ops (id <= lastId) rather than flagging them. A synced op has
  // no further use — the durable kv_store/meta_store record committed alongside it is the
  // source of truth — and retaining flagged rows grew op_log unboundedly across all sessions.
  private async markOpsSyncedInternal(lastId: number): Promise<void> {
    const tx = this.db?.transaction('op_log', 'readwrite');
    if (!tx) return;
    await tx.store.delete(IDBKeyRange.upperBound(lastId));
    await tx.done;
  }

  async deleteOp(id: number): Promise<void> {
    return this.queueOrExecute('deleteOp', [id], () => this.deleteOpInternal(id));
  }

  private async deleteOpInternal(id: number): Promise<void> {
    await this.db?.delete('op_log', id);
  }

  async commitWrite(mutations: StorageMutation[], op: Omit<OpLogEntry, 'id'>): Promise<number> {
    return this.queueOrExecute('commitWrite', [mutations, op], () =>
      this.commitWriteInternal(mutations, op),
    );
  }

  // Crash-consistent local write: apply every KV/meta mutation AND append the op log entry
  // in ONE readwrite transaction spanning all affected stores, so a record and its pending
  // op are never observable independently (no record-without-op, no op-without-record).
  private async commitWriteInternal(
    mutations: StorageMutation[],
    op: Omit<OpLogEntry, 'id'>,
  ): Promise<number> {
    if (!this.db) {
      throw new Error('IDBAdapter.commitWrite called before database is ready');
    }
    const storeNames = new Set<'kv_store' | 'meta_store'>(['kv_store']);
    for (const m of mutations) {
      storeNames.add(m.store === 'meta' ? 'meta_store' : 'kv_store');
    }
    const tx = this.db.transaction([...storeNames, 'op_log'], 'readwrite');
    for (const m of mutations) {
      const store = tx.objectStore(m.store === 'meta' ? 'meta_store' : 'kv_store');
      if (m.type === 'remove') {
        await store.delete(m.key);
      } else {
        await store.put({ key: m.key, value: m.value });
      }
    }
    const id = (await tx.objectStore('op_log').add({ ...op, synced: 0 })) as number;
    await tx.done;
    return id;
  }
}
