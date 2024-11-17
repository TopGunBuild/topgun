import { windowOrGlobal } from '@topgunbuild/common';
import { symmetric, Password } from '@topgunbuild/crypto';
import { StorageAdapter, StorageParams } from './types';

/**
 * IndexedDBStorage is a storage adapter that uses IndexedDB to store data.
 * @template T The type of the data to store
 */
export class IndexedDBStorage<T> implements StorageAdapter<T> {
    private _dbp: Promise<IDBDatabase> | undefined;
    private _encryptionKey?: Password;
    readonly _dbName: string;
    readonly _storeName: string;
    readonly _writeMiddleware?: (value: T) => Uint8Array;
    readonly _readMiddleware?: (value: Uint8Array) => T;

    /**
     * Check if a database exists
     * @param dbName The name of the database to check
     * @returns Promise that resolves to true if database exists, false otherwise
     */
    static async exists(dbName: string): Promise<boolean> {
        if (!this.isSupported()) {
            return false;
        }

        return new Promise((resolve) => {
            const request = windowOrGlobal.indexedDB.open(dbName);
            let exists = true;

            request.onupgradeneeded = () => {
                exists = false;
                const db = request.result;
                db.close();
                windowOrGlobal.indexedDB.deleteDatabase(dbName);
            };

            request.onsuccess = () => {
                const db = request.result;
                db.close();
                resolve(exists);
            };

            request.onerror = () => {
                resolve(false);
            };
        });
    }

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
    constructor(params: StorageParams<T, Uint8Array>) {
        this._dbName = params.dbName || 'topgun';
        this._storeName = params.storeName || 'storage';
        this._encryptionKey = params.encryptionKey;
        this._writeMiddleware = params.writeMiddleware;
        this._readMiddleware = params.readMiddleware;
        this.init();
    }

    /**
     * Get a value from the storage and decrypt if encryption is enabled
     * @param key 
     * @returns 
     */
    public async get(key: IDBValidKey): Promise<T> {
        const db = await this._dbp;
        const transaction = db.transaction(this._storeName, 'readonly');
        const store = transaction.objectStore(this._storeName);
        const result = await new Promise<any>((resolve, reject) => {
            const request = store.get(key);
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => reject(request.error);
        });

        if (!result) return result;

        if (this._encryptionKey) {
            const decrypted = symmetric.decryptBytes(result, this._encryptionKey);
            return this._readMiddleware ? this._readMiddleware(decrypted) : decrypted as T;
        }

        return this._readMiddleware ? this._readMiddleware(result) : result as T;
    }

    /**
     * Encrypt value if encryption is enabled and store it
     * @param key 
     * @param value 
     * @returns 
     */
    public async put(key: IDBValidKey, value: T): Promise<void> {
        const db = await this._dbp;
        const transaction = db.transaction(this._storeName, 'readwrite');
        const store = transaction.objectStore(this._storeName);

        return new Promise((resolve, reject) => {
            transaction.oncomplete = () => resolve();
            transaction.onerror = () => reject(transaction.error);

            if (value === null) {
                store.delete(key);
            } else {
                let valueToStore = this._writeMiddleware
                    ? this._writeMiddleware(value)
                    : value;
                
                if (this._encryptionKey) {
                    valueToStore = symmetric.encryptBytes(valueToStore as Uint8Array, this._encryptionKey);
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
    public delete(key: IDBValidKey): Promise<void> {
        return this.withIDBStore('readwrite', (store) => {
            store.delete(key);
        });
    }

    /**
     * Get all values from the storage
     * @returns 
     */
    public async getAll(): Promise<Record<string, T>> {
        const db = await this._dbp;
        const transaction = db.transaction(this._storeName, 'readonly');
        const store = transaction.objectStore(this._storeName);
        
        const results = await new Promise<any[]>((resolve, reject) => {
            const request = store.getAll();
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => reject(request.error);
        });

        return results.reduce((acc, curr) => {
            let value = curr.value;
            if (this._encryptionKey) {
                value = symmetric.decryptBytes(value, this._encryptionKey);
            }
            acc[curr.key] = this._readMiddleware
                ? this._readMiddleware(value)
                : value as T;
            return acc;
        }, {} as Record<string, T>);
    }

    /**
     * Close the database
     */
    public async close() {
        const db = await this._dbp;
        db.close();
    }

    /**
     * Initialize the storage
     */
    private init(): void {
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
    private withIDBStore(
        type: IDBTransactionMode,
        callback: (store: IDBObjectStore) => void,
    ): Promise<void> {
        this.init();
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
