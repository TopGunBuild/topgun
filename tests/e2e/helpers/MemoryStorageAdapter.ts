import { LWWRecord, ORMapRecord } from '@topgunbuild/core';

export interface OpLogEntry {
  id?: number;
  key: string;
  op: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE';
  value?: any;
  record?: LWWRecord<any>;
  orRecord?: ORMapRecord<any>;
  orTag?: string;
  hlc?: string;
  timestamp?: any;
  synced: number;
  mapName: string;
}

export interface IStorageAdapter {
  initialize(dbName: string): Promise<void>;
  close(): Promise<void>;
  get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined>;
  put(key: string, value: any): Promise<void>;
  remove(key: string): Promise<void>;
  getMeta(key: string): Promise<any>;
  setMeta(key: string, value: any): Promise<void>;
  batchPut(entries: Map<string, any>): Promise<void>;
  appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number>;
  getPendingOps(): Promise<OpLogEntry[]>;
  markOpsSynced(lastId: number): Promise<void>;
  getAllKeys(): Promise<string[]>;
}

export class MemoryStorageAdapter implements IStorageAdapter {
  private data = new Map<string, any>();
  private meta = new Map<string, any>();
  private opLog: OpLogEntry[] = [];
  private opLogIdCounter = 1;

  async initialize(_dbName: string): Promise<void> {
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

  // Helper for tests
  clear(): void {
    this.data.clear();
    this.meta.clear();
    this.opLog = [];
    this.opLogIdCounter = 1;
  }
}
