import { windowOrGlobal } from '@topgunbuild/utils';
import { StorageAdapter } from './types';

/**
 * IndexedDBStorage is a storage adapter that uses IndexedDB to store data.
 * @template T The type of the data to store
 */
export class IndexedDBStorage<T> implements StorageAdapter<T> {
    private _dbp: Promise<IDBDatabase> | undefined;
    readonly _dbName: string;
    readonly _storeName: string;

    /**
     * Check if IndexedDB is supported
     * @returns 
     */
    static isSupported(): boolean {
        return !!windowOrGlobal?.indexedDB;
    }

    /**
     * Create a new IndexedDBStorage
     * @param params 
     */
    constructor(params: { dbName?: string, storeName?: string }) {
        this._dbName = params.dbName || 'topgun';
        this._storeName = params.storeName || 'storage';
        this.#init();
    }

    /**
     * Get a value from the storage
     * @param key 
     * @returns 
     */
    get(key: IDBValidKey): Promise<T> {
        let req: IDBRequest;
        return this.#withIDBStore('readwrite', (store) => {
            req = store.get(key);
        }).then(() => req.result);
    }

    /**
     * Put a value into the storage
     * @param key 
     * @param value 
     * @returns 
     */
    put(key: IDBValidKey, value: any): Promise<void> {
        return this.#withIDBStore('readwrite', (store) => {
            if (value === null) {
                store.delete(key);
            }
            else {
                store.put(value, key);
            }
        });
    }

    /**
     * Delete a value from the storage
     * @param key 
     * @returns 
     */
    delete(key: IDBValidKey): Promise<void> {
        return this.#withIDBStore('readwrite', (store) => {
            store.delete(key);
        });
    }

    /**
     * Get all values from the storage
     * @returns 
     */
    getAll(): Promise<Record<string, T>> {
        let req: IDBRequest;
        return this.#withIDBStore('readwrite', (store) => {
            req = store.getAll();
        }).then(() => req.result.reduce((acc, curr) => {
            acc[curr.key] = curr.value;
            return acc;
        }, {} as Record<string, T>));
    }

    /**
     * Update a value in the storage
     * @param key 
     * @param updater 
     * @returns 
     */
    update(key: IDBValidKey, updater: (val: Partial<T>) => T): Promise<void> {
        return this.#withIDBStore('readwrite', (store) => {
            const req = store.get(key);
            req.onsuccess = () => {
                store.put(updater(req.result), key);
            };
        });
    }

    /**
     * Initialize the storage
     */
    #init(): void {
        if (this._dbp) {
            return;
        }
        this._dbp = new Promise((resolve, reject) => {
            const db = windowOrGlobal?.indexedDB;

            if (!db) {
                return reject(Error('IndexedDB could not be found!'));
            }

            const openreq = db.open(this._dbName);
            openreq.onerror = () => reject(openreq.error);
            openreq.onsuccess = () => resolve(openreq.result);

            // First time setup: create an empty object store
            openreq.onupgradeneeded = () => {
                openreq.result.createObjectStore(this._storeName);
            };
        });
    }

    /**
     * Execute a callback with an IDBObjectStore
     * @param type 
     * @param callback 
     * @returns 
     */
    #withIDBStore(
        type: IDBTransactionMode,
        callback: (store: IDBObjectStore) => void,
    ): Promise<void> {
        this.#init();
        return (this._dbp as Promise<IDBDatabase>).then(
            db =>
                new Promise<void>((resolve, reject) => {
                    const transaction = db.transaction(this._storeName, type);
                    transaction.oncomplete = () => resolve();
                    transaction.onabort = transaction.onerror = () =>
                        reject(transaction.error);
                    callback(transaction.objectStore(this._storeName));
                }),
        );
    }
}
