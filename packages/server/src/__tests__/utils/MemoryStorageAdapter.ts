import { LWWRecord } from '@topgunbuild/core';
// We are importing from client source for testing purposes
import { IStorageAdapter, OpLogEntry } from '../../../../client/src/IStorageAdapter';

export class MemoryStorageAdapter implements IStorageAdapter {
  private data = new Map<string, any>();
  private meta = new Map<string, any>();
  private opLog: OpLogEntry[] = [];
  private opLogIdCounter = 1;

  async initialize(dbName: string): Promise<void> {
    // no-op
  }

  async close(): Promise<void> {
    this.data.clear();
    this.meta.clear();
    this.opLog = [];
  }

  async get<V>(key: string): Promise<LWWRecord<V> | undefined> {
    return this.data.get(key);
  }

  async put<V>(key: string, record: LWWRecord<V>): Promise<void> {
    this.data.set(key, record);
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

  async batchPut<V>(entries: Map<string, LWWRecord<V>>): Promise<void> {
    for (const [key, record] of entries) {
      this.data.set(key, record);
    }
  }

  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opLogIdCounter++;
    this.opLog.push({ ...entry, id } as OpLogEntry);
    return id;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.opLog.filter(op => !op.synced);
  }

  async markOpsSynced(lastId: number): Promise<void> {
    for (const op of this.opLog) {
      if (op.id && op.id <= lastId) {
        op.synced = 1;
      }
    }
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.data.keys());
  }
}

