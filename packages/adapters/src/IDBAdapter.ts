import { LWWRecord, ORMapRecord } from '@topgunbuild/core';
import { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import { openDB, IDBPDatabase } from 'idb';

export class IDBAdapter implements IStorageAdapter {
  private dbPromise?: Promise<IDBPDatabase>;
  private dbName: string = '';

  async initialize(dbName: string): Promise<void> {
    this.dbName = dbName;
    this.dbPromise = openDB(dbName, 2, { // Bump version for new stores if needed, though we reuse kv_store
      upgrade(db, oldVersion, newVersion, transaction) {
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
    await this.dbPromise;
  }

  async close(): Promise<void> {
    // No-op
  }

  async get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined> {
    const db = await this.dbPromise;
    const result = await db?.get('kv_store', key);
    return result?.value; // We store { key, value: ... }
  }

  async put(key: string, value: any): Promise<void> {
    const db = await this.dbPromise;
    await db?.put('kv_store', { key, value });
  }

  async remove(key: string): Promise<void> {
    const db = await this.dbPromise;
    await db?.delete('kv_store', key);
  }

  async getMeta(key: string): Promise<any> {
      const db = await this.dbPromise;
      const result = await db?.get('meta_store', key);
      return result?.value;
  }

  async setMeta(key: string, value: any): Promise<void> {
      const db = await this.dbPromise;
      await db?.put('meta_store', { key, value });
  }

  async batchPut(entries: Map<string, any>): Promise<void> {
    const db = await this.dbPromise;
    const tx = db?.transaction('kv_store', 'readwrite');
    if (!tx) return;

    await Promise.all(
      Array.from(entries.entries()).map(([key, value]) =>
        tx.store.put({ key, value })
      )
    );
    await tx.done;
  }

  async appendOpLog(entry: any): Promise<number> {
    const db = await this.dbPromise;
    // Ensure synced is 0
    const entryToSave = { ...entry, synced: 0 };
    return await db?.add('op_log', entryToSave) as number;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    const db = await this.dbPromise;
    const all = await db?.getAll('op_log');
    return all?.filter((op: any) => op.synced === 0) || [];
  }

  async markOpsSynced(lastId: number): Promise<void> {
    const db = await this.dbPromise;
    const tx = db?.transaction('op_log', 'readwrite');
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

  async getAllKeys(): Promise<string[]> {
    const db = await this.dbPromise;
    return (await db?.getAllKeys('kv_store')) as string[] || [];
  }
}
