import { windowOrGlobal } from '@topgunbuild/common';
import { symmetric, Password } from '@topgunbuild/crypto';
import { StorageAdapter, StorageParams } from './types';

/**
 * IndexedDBStorage is a storage adapter that uses IndexedDB to store data.
 * @template T The type of the data to store
 */
export class IndexedDBStorage implements StorageAdapter<Uint8Array> {
    private _dbp: Promise<IDBDatabase> | undefined;
    private _encryptionKey?: Password;
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
    constructor(params: StorageParams) {
        this._dbName = params.dbName || 'topgun';
        this._storeName = params.storeName || 'storage';
        this._encryptionKey = params.encryptionKey;
        this.#init();
    }

    /**
     * Get a value from the storage and decrypt if encryption is enabled
     * @param key 
     * @returns 
     */
    async get(key: IDBValidKey): Promise<Uint8Array> {
        let req: IDBRequest;
        await this.#withIDBStore('readwrite', (store) => {
            req = store.get(key);
        });

        if (!req.result) return req.result;

        // Decrypt the value if encryption is enabled
        if (this._encryptionKey) {
            return symmetric.decryptBytes(req.result, this._encryptionKey);
        }

        return req.result;
    }

    /**
     * Encrypt value if encryption is enabled and store it
     * @param key 
     * @param value 
     * @returns 
     */
    async put(key: IDBValidKey, value: Uint8Array): Promise<void> {
        return this.#withIDBStore('readwrite', (store) => {
            if (value === null) {
                store.delete(key);
            } else {
                let valueToStore = value;
                
                // Encrypt the value if encryption is enabled
                if (this._encryptionKey) {
                    valueToStore = symmetric.encryptBytes(value, this._encryptionKey);
                }

                store.put(valueToStore, key);
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
    getAll(): Promise<Record<string, Uint8Array>> {
        let req: IDBRequest;
        return this.#withIDBStore('readwrite', (store) => {
            req = store.getAll();
        }).then(() => req.result.reduce((acc, curr) => {
            let value = curr.value;
            // Decrypt the value if encryption is enabled
            if (this._encryptionKey) {
                value = symmetric.decryptBytes(value, this._encryptionKey);
            }
            acc[curr.key] = value;
            return acc;
        }, {} as Record<string, Uint8Array>));
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
