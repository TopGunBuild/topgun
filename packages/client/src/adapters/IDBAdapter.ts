import type { LWWRecord, ORMapRecord } from '@topgunbuild/core';
import type { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { openDB } from 'idb';
import type { IDBPDatabase } from 'idb';

/**
 * Represents an operation queued before IndexedDB is ready.
 */
interface QueuedOperation {
  type: 'put' | 'remove' | 'setMeta' | 'appendOpLog' | 'markOpsSynced' | 'batchPut';
  args: any[];
  resolve: (value: any) => void;
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
    // Start initialization but don't await it
    this.initPromise = this.initializeInternal(dbName);
    // Return immediately - non-blocking!
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

      // Replay queued operations
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
    args: any[],
    executor: () => Promise<T>
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

  async get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined> {
    // Read operations must wait for DB to be ready
    await this.waitForReady();
    const result = await this.db?.get('kv_store', key);
    return result?.value;
  }

  async getMeta(key: string): Promise<any> {
    await this.waitForReady();
    const result = await this.db?.get('meta_store', key);
    return result?.value;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    await this.waitForReady();
    const all = await this.db?.getAll('op_log');
    return all?.filter((op: any) => op.synced === 0) || [];
  }

  async getAllKeys(): Promise<string[]> {
    await this.waitForReady();
    return (await this.db?.getAllKeys('kv_store')) as string[] || [];
  }

  // ============================================
  // Write Operations - Queue if not ready
  // ============================================

  async put(key: string, value: any): Promise<void> {
    return this.queueOrExecute('put', [key, value], () => this.putInternal(key, value));
  }

  private async putInternal(key: string, value: any): Promise<void> {
    await this.db?.put('kv_store', { key, value });
  }

  async remove(key: string): Promise<void> {
    return this.queueOrExecute('remove', [key], () => this.removeInternal(key));
  }

  private async removeInternal(key: string): Promise<void> {
    await this.db?.delete('kv_store', key);
  }

  async setMeta(key: string, value: any): Promise<void> {
    return this.queueOrExecute('setMeta', [key, value], () => this.setMetaInternal(key, value));
  }

  private async setMetaInternal(key: string, value: any): Promise<void> {
    await this.db?.put('meta_store', { key, value });
  }

  async batchPut(entries: Map<string, any>): Promise<void> {
    return this.queueOrExecute('batchPut', [entries], () => this.batchPutInternal(entries));
  }

  private async batchPutInternal(entries: Map<string, any>): Promise<void> {
    const tx = this.db?.transaction('kv_store', 'readwrite');
    if (!tx) return;

    await Promise.all(
      Array.from(entries.entries()).map(([key, value]) =>
        tx.store.put({ key, value })
      )
    );
    await tx.done;
  }

  async appendOpLog(entry: any): Promise<number> {
    return this.queueOrExecute('appendOpLog', [entry], () => this.appendOpLogInternal(entry));
  }

  private async appendOpLogInternal(entry: any): Promise<number> {
    const entryToSave = { ...entry, synced: 0 };
    return await this.db?.add('op_log', entryToSave) as number;
  }

  async markOpsSynced(lastId: number): Promise<void> {
    return this.queueOrExecute('markOpsSynced', [lastId], () => this.markOpsSyncedInternal(lastId));
  }

  private async markOpsSyncedInternal(lastId: number): Promise<void> {
    const tx = this.db?.transaction('op_log', 'readwrite');
    if (!tx) return;

    let cursor = await tx.store.openCursor();
    while (cursor) {
      if (cursor.value.id <= lastId) {
        const update = { ...cursor.value, synced: 1 };
        await cursor.update(update);
      }
      cursor = await cursor.continue();
    }
    await tx.done;
  }
}

