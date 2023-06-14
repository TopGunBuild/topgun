import { isNumber, isString } from 'topgun-typed';
import { StorageListOptions, TGStorage } from '../storage';
import { filterNodesByListOptions, lexicographicCompare, listFilterMatch } from '../storage/utils';
import { TGNode } from '../types';

export class IndexedDBStorage implements TGStorage
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

    async list<Type>(options: StorageListOptions): Promise<Type>
    {
        const allNodes = await this.getAll();
        const nodes    = filterNodesByListOptions(allNodes, options);

        return nodes.reduce((accum: Type, node: TGNode) => ({ ...accum, [node._['#']]: node }), {} as Type);
    }

    put(key: IDBValidKey, value: any): Promise<void>
    {
        return this._withIDBStore('readwrite', (store) =>
        {
            store.put(value, key);
        });
    }

    getAll(): Promise<TGNode[]>
    {
        let req: IDBRequest;
        return this._withIDBStore('readwrite', (store) =>
        {
            req = store.getAll();
        }).then(() => req.result);
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
