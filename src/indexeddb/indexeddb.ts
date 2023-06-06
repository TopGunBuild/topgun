import { TGStorage } from '../types';

export class IndexedDb implements TGStorage
{
    private _dbp: Promise<IDBDatabase>|undefined;
    readonly _dbName: string;
    readonly _storeName: string;

    /**
     * Constructor
     */
    constructor(storeName: string)
    {
        this._dbName    = storeName;
        this._storeName = storeName;
        this._init();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    get<Type>(key: IDBValidKey): Promise<Type>
    {
        let req: IDBRequest;
        return this._withIDBStore('readwrite', (store) =>
        {
            req = store.get(key);
        }).then(() => req.result);
    }

    getALlKeys(): Promise<string[]>
    {
        let req: IDBRequest;
        return this._withIDBStore('readwrite', (store) =>
        {
            req = store.getAll();
        }).then(() => req.result);
    }

    put(key: IDBValidKey, value: any): Promise<void>
    {
        return this._withIDBStore('readwrite', (store) =>
        {
            store.put(value, key);
        });
    }

    update(key: IDBValidKey, updater: (val: any) => any): Promise<void>
    {
        return this._withIDBStore('readwrite', (store) =>
        {
            const req     = store.get(key);
            req.onsuccess = () =>
            {
                store.put(updater(req.result), key);
            };
        });
    }

    del(key: IDBValidKey): Promise<void>
    {
        return this._withIDBStore('readwrite', (store) =>
        {
            store.delete(key);
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _init(): void
    {
        if (this._dbp)
        {
            return;
        }
        this._dbp = new Promise((resolve, reject) =>
        {
            const indexedDB =
                      window.indexedDB ||
                      window['mozIndexedDB'] ||
                      window['webkitIndexedDB'] ||
                      window['msIndexedDB'] ||
                      window['shimIndexedDB'];

            if (!indexedDB)
            {
                return reject(Error('IndexedDB could not be found!'));
            }

            const openreq     = indexedDB.open(this._dbName);
            openreq.onerror   = () => reject(openreq.error);
            openreq.onsuccess = () => resolve(openreq.result);

            // First time setup: create an empty object store
            openreq.onupgradeneeded = () =>
            {
                openreq.result.createObjectStore(this._storeName);
            };
        });
    }

    private _withIDBStore(
        type: IDBTransactionMode,
        callback: (store: IDBObjectStore) => void,
    ): Promise<void>
    {
        this._init();
        return (this._dbp as Promise<IDBDatabase>).then(
            db =>
                new Promise<void>((resolve, reject) =>
                {
                    const transaction      = db.transaction(this._storeName, type);
                    transaction.oncomplete = () => resolve();
                    transaction.onabort    = transaction.onerror = () =>
                        reject(transaction.error);
                    callback(transaction.objectStore(this._storeName));
                }),
        );
    }
}
