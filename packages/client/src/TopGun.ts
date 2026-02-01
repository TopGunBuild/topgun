import { TopGunClient } from './TopGunClient';
import { IDBAdapter } from './adapters/IDBAdapter';
import type { IStorageAdapter } from './IStorageAdapter';
import { LWWMap } from '@topgunbuild/core';
import type { LWWRecord } from '@topgunbuild/core';
import { logger } from './utils/logger';

export interface TopGunConfig {
  sync: string;
  persist: 'indexeddb' | IStorageAdapter;
  nodeId?: string;
}

// Generic schema type
export type TopGunSchema = Record<string, any>;

const handler: ProxyHandler<TopGun<any>> = {
  get(target, prop, receiver) {
    if (prop in target || typeof prop === 'symbol') {
      return Reflect.get(target, prop, receiver);
    }
    if (typeof prop === 'string') {
        return target.collection(prop);
    }
    return undefined;
  }
};

export class TopGun<T extends TopGunSchema = any> {
  private client: TopGunClient;
  private initPromise: Promise<void>;
  
  // Allow property access for collections based on Schema T
  [key: string]: any;

  constructor(config: TopGunConfig) {
    let storage: IStorageAdapter;

    if (config.persist === 'indexeddb') {
       storage = new IDBAdapter();
    } else if (typeof config.persist === 'object') {
      storage = config.persist;
    } else {
       throw new Error(`Unsupported persist option: ${config.persist}`);
    }

    this.client = new TopGunClient({
      serverUrl: config.sync,
      storage,
      nodeId: config.nodeId
    });

    // Start client initialization (non-blocking)
    // The IDBAdapter now initializes in the background and queues operations
    this.initPromise = this.client.start().catch(err => {
        logger.error({ err, context: 'client_start' }, 'Failed to start TopGun client');
        throw err;
    });

    return new Proxy(this, handler);
  }

  /**
   * Waits for the storage adapter to be fully initialized.
   * This is optional - you can start using the database immediately.
   * Operations are queued in memory and persisted once IndexedDB is ready.
   */
  public async waitForReady(): Promise<void> {
      await this.initPromise;
  }

  public collection<K extends keyof T & string>(name: K): CollectionWrapper<T[K]> {
    // Explicitly type the map
    const map = this.client.getMap<string, T[K]>(name);
    return new CollectionWrapper<T[K]>(map);
  }
}

export class CollectionWrapper<ItemType = any> {
  private map: LWWMap<string, ItemType>;

  constructor(map: LWWMap<string, ItemType>) {
    this.map = map;
  }

  /**
   * Sets an item in the collection. 
   * The item MUST have an 'id' or '_id' field.
   */
  async set(value: ItemType): Promise<ItemType> {
     const v = value as any;
     const key = v.id || v._id;
     if (!key) {
         throw new Error('Object must have an "id" or "_id" property to be saved in a collection.');
     }
     
     // LWWMap.set is synchronous in updating memory and queueing ops,
     // but we return a Promise to match typical async DB APIs.
     this.map.set(key, value);
     return Promise.resolve(value); 
  }
  
  /**
   * Retrieves an item by ID.
   * Returns the value directly (unwrapped from CRDT record).
   */
  get(key: string): ItemType | undefined {
      return this.map.get(key);
  }

  /**
   * Get the raw LWWRecord (including metadata like timestamp).
   */
  getRecord(key: string): LWWRecord<ItemType> | undefined {
      return this.map.getRecord(key);
  }

  // Expose raw map if needed for advanced usage
  get raw() {
      return this.map;
  }
}
