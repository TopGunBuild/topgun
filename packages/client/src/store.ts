import { ClientConfig, NetworkListenerAdapter, QueryCb, QueryState } from "./types";
import { IndexedDBStorage } from "./storage/indexeddb-storage";
import { WebSocketManager } from "./websocket";
import { WindowNetworkListener } from "./utils/window-network-listener";
import { toHexString, windowOrGlobal } from "@topgunbuild/utils";
import { Action, SelectQuery } from "@topgunbuild/types";
import { MemoryStorage } from "./storage/memory-storage";
import { StorageManager } from "./storage/storage-manager";

/**
 * The Store class is the main entry point for the TopGun client library.
 * It manages the connection to the websocket servers and the storage of data.
 */
export class Store {
    private websocketManager: WebSocketManager;
    private storageManager: StorageManager;
    private config: ClientConfig;
    private networkListener: NetworkListenerAdapter;
    private isOnline: boolean;
    private beforeUnloadCbs: (() => void)[] = [];
    private queryCbs: Record<string, QueryState<any>> = {};

    /**
     * Create a new Store
     * @param config The configuration for the store
     */
    constructor(config: ClientConfig) {
        this.config = config;

        this.initWebSocketManagers();
        this.initStorageManager();
        this.initNetworkListener();
        this.initBeforeUnload();
    }

    /**
     * Subscribe to a query
     * @param query The query to execute
     * @param cb The callback to call with the result
     */
    public subscribeQuery<T>(query: SelectQuery, cb: QueryCb<T>): () => void {
        const encodedQuery = query.encode();
        const queryHash = toHexString(encodedQuery);
        if (!this.queryCbs[queryHash]) {
            this.queryCbs[queryHash] = {
                query,
                cbs: [],
                result: null,
                resultHash: null,
            };
            // Send action to all websocket managers for first action
            this.websocketManager.send(encodedQuery);
        }
        this.queryCbs[queryHash].cbs.push(cb);
        this.storageManager.putQuery(queryHash, query);

        return () => {
            this.unsubscribeQuery(query, cb, queryHash);
        };
    }

    /**
     * Unsubscribe from a query
     * @param action The query to unsubscribe from
     * @param cb The callback to unsubscribe
     * @param actionHash The hash of the action
     */
    public unsubscribeQuery(query: SelectQuery, cb: QueryCb<any>, queryHash?: string) {
        if (!queryHash) {
            const encodedQuery = query.encode();
            queryHash = toHexString(encodedQuery);
        }

        this.queryCbs[queryHash].cbs = this.queryCbs[queryHash].cbs.filter(q => q !== cb);
        if (this.queryCbs[queryHash].cbs.length === 0) {
            delete this.queryCbs[queryHash];
            this.storageManager.deleteQuery(queryHash);
        }
    }

    /**
     * Add a callback to be called before the window is unloaded
     * @param cb The callback to call
     */
    public beforeUnload(cb: () => void) {
        this.beforeUnloadCbs.push(cb);
    }

    /**
     * Initialize the websocket managers
     */
    private initWebSocketManagers() {
        this.websocketManager = new WebSocketManager({ websocketURI: this.config.websocketURI, appId: this.config.appId });
    }

    /**
     * Initialize the storage manager
     */
    private initStorageManager() {
        const dbName = `topgun-${this.config.appId}`;

        if (this.config.storage) {
            this.storageManager = new StorageManager(dbName, this.config.storage);
        }
        else if (IndexedDBStorage.isSupported()) {
            this.storageManager = new StorageManager(dbName, IndexedDBStorage);
        }
        else {
            this.storageManager = new StorageManager(dbName, MemoryStorage);
        }
    }

    /**
     * Initialize the network listener
     */
    private initNetworkListener() {
        this.networkListener = this.config.windowNetworkListener
            ? new this.config.windowNetworkListener()
            : new WindowNetworkListener();
        this.isOnline = this.networkListener.isOnline();
        if (this.isOnline) {
            this.websocketManager.connect();
        }

        this.networkListener.listen((isOnline) => {
            if (isOnline === this.isOnline) {
                return;
            }
            this.isOnline = isOnline;
            if (isOnline) {
                this.websocketManager.connect();
            } else {
                this.websocketManager.disconnect();
            }
        });
    }

    /**
     * Initialize the before unload event
     */
    private initBeforeUnload() {
        windowOrGlobal?.addEventListener("beforeunload", () => {
            this.beforeUnloadCbs.forEach(cb => cb());
        });
    }
}
