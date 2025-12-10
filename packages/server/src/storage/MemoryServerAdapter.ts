import { IServerStorage, StorageValue } from './IServerStorage';

/**
 * In-memory implementation of IServerStorage.
 * Useful for development, testing, and demos without requiring a database.
 *
 * Note: Data is lost when the server restarts.
 */
export class MemoryServerAdapter implements IServerStorage {
  // Map<mapName, Map<key, value>>
  private storage = new Map<string, Map<string, StorageValue<any>>>();

  async initialize(): Promise<void> {
    // No-op for in-memory storage
    console.log('[MemoryServerAdapter] Initialized in-memory storage');
  }

  async close(): Promise<void> {
    this.storage.clear();
    console.log('[MemoryServerAdapter] Storage cleared and closed');
  }

  private getMap(mapName: string): Map<string, StorageValue<any>> {
    let map = this.storage.get(mapName);
    if (!map) {
      map = new Map();
      this.storage.set(mapName, map);
    }
    return map;
  }

  async load(mapName: string, key: string): Promise<StorageValue<any> | undefined> {
    return this.getMap(mapName).get(key);
  }

  async loadAll(mapName: string, keys: string[]): Promise<Map<string, StorageValue<any>>> {
    const map = this.getMap(mapName);
    const result = new Map<string, StorageValue<any>>();
    for (const key of keys) {
      const value = map.get(key);
      if (value !== undefined) {
        result.set(key, value);
      }
    }
    return result;
  }

  async loadAllKeys(mapName: string): Promise<string[]> {
    return Array.from(this.getMap(mapName).keys());
  }

  async store(mapName: string, key: string, record: StorageValue<any>): Promise<void> {
    this.getMap(mapName).set(key, record);
  }

  async storeAll(mapName: string, records: Map<string, StorageValue<any>>): Promise<void> {
    const map = this.getMap(mapName);
    for (const [key, value] of records) {
      map.set(key, value);
    }
  }

  async delete(mapName: string, key: string): Promise<void> {
    this.getMap(mapName).delete(key);
  }

  async deleteAll(mapName: string, keys: string[]): Promise<void> {
    const map = this.getMap(mapName);
    for (const key of keys) {
      map.delete(key);
    }
  }
}
