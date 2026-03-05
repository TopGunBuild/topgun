import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';

/**
 * In-memory storage adapter for the Sync Lab demo.
 * Each "device" gets its own instance so state is fully isolated
 * without touching IndexedDB or any persistent store.
 */
export class MemoryStorageAdapter implements IStorageAdapter {
  private data = new Map<string, any>();
  private meta = new Map<string, any>();
  private opLog: (OpLogEntry & { id: number })[] = [];
  private nextId = 1;

  async initialize(_dbName: string): Promise<void> {
    // Nothing to initialize for in-memory storage
  }

  async close(): Promise<void> {
    this.data.clear();
    this.meta.clear();
    this.opLog = [];
  }

  async get<V>(key: string): Promise<V | undefined> {
    return this.data.get(key);
  }

  async put(key: string, value: any): Promise<void> {
    this.data.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.data.delete(key);
  }

  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }

  async setMeta(key: string, value: any): Promise<void> {
    this.meta.set(key, value);
  }

  async batchPut(entries: Map<string, any>): Promise<void> {
    entries.forEach((v, k) => this.data.set(k, v));
  }

  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.nextId++;
    this.opLog.push({ ...entry, id } as OpLogEntry & { id: number });
    return id;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.opLog.filter(e => e.synced === 0);
  }

  async markOpsSynced(lastId: number): Promise<void> {
    this.opLog.forEach(e => {
      if (e.id <= lastId) e.synced = 1;
    });
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.data.keys());
  }
}
