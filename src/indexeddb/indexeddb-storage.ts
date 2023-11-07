import global from '../utils/window-or-global';
import { TGStorage } from '../storage';
import { arrayNodesToObject, filterNodes } from '../storage/utils';
import { TGGraphData, TGNode, TGOptionsGet } from '../types';

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
        this.#init();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    get(key: IDBValidKey): Promise<TGNode>
    {
        let req: IDBRequest;
        return this.#withIDBStore('readwrite', (store) =>
        {
            req = store.get(key);
        }).then(() => req.result);
    }

    async list(options: TGOptionsGet): Promise<TGGraphData>
    {
        const allNodes = await this.getAll();
        const nodes    = filterNodes(allNodes, options);

        return arrayNodesToObject(nodes);
    }

    put(key: IDBValidKey, value: any): Promise<void>
    {
        return this.#withIDBStore('readwrite', (store) =>
        {
            store.put(value, key);
        });
    }

    getAll(): Promise<TGNode[]>
    {
        let req: IDBRequest;
        return this.#withIDBStore('readwrite', (store) =>
        {
            req = store.getAll();
        }).then(() => req.result);
    }

    update(key: IDBValidKey, updater: (val: any) => any): Promise<void>
    {
        return this.#withIDBStore('readwrite', (store) =>
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

    #init(): void
    {
        if (this._dbp)
        {
            return;
        }
        this._dbp = new Promise((resolve, reject) =>
        {
            const indexedDB =
                      global.indexedDB ||
                      global['mozIndexedDB'] ||
                      global['webkitIndexedDB'] ||
                      global['msIndexedDB'] ||
                      global['shimIndexedDB'];

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

    #withIDBStore(
        type: IDBTransactionMode,
        callback: (store: IDBObjectStore) => void,
    ): Promise<void>
    {
        this.#init();
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
