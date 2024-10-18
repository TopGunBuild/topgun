import { ClientConfig, NetworkListenerAdapter, QueryCb, QueryState, StorageAdapter } from "./types";
import { IndexedDBStorage } from "./utils/indexdb-storage";
import { WebSocketManager } from "./websocket";
import { WindowNetworkListener } from "./utils/window-network-listener";
import { toHexString, windowOrGlobal } from "@topgunbuild/utils";
import { Action } from "@topgunbuild/types";

/**
 * The Store class is the main entry point for the TopGun client library.
 * It manages the connection to the websocket servers and the storage of data.
 */
export class Store {
    private websocketManagers: WebSocketManager[];
    private queryStorage: StorageAdapter<any>;
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
        this.initQueryStorage();
        this.initWebSocketManagers();
        this.initNetworkListener();
        this.initBeforeUnload();
    }

    /**
     * Subscribe to a query
     * @param query The query to execute
     * @param cb The callback to call with the result
     */
    public subscribeQuery<T>(action: Action, cb: QueryCb<T>): () => void {
        const encodedAction = action.encode();
        const actionHash = toHexString(encodedAction);
        if (!this.queryCbs[actionHash]) {
            this.queryCbs[actionHash] = {
                action,
                cbs: [],
                result: null,
                resultHash: null,
            };
            // Send action to all websocket managers for first action
            this.websocketManagers.forEach(manager => {
                manager.send(encodedAction);
            });
        }
        this.queryCbs[actionHash].cbs.push(cb);
        this.queryStorage.put(actionHash, action);

        return () => {
            this.unsubscribeQuery(action, cb, actionHash);
        };
    }

    /**
     * Unsubscribe from a query
     * @param action The query to unsubscribe from
     * @param cb The callback to unsubscribe
     * @param actionHash The hash of the action
     */
    public unsubscribeQuery(action: Action, cb: QueryCb<any>, actionHash?: string) {
        if (!actionHash) {
            const encodedAction = action.encode();
            actionHash = toHexString(encodedAction);
        }

        this.queryCbs[actionHash].cbs = this.queryCbs[actionHash].cbs.filter(q => q !== cb);
        if (this.queryCbs[actionHash].cbs.length === 0) {
            delete this.queryCbs[actionHash];
            this.queryStorage.delete(actionHash);
        }
    }

    /**
     * Initialize the websocket managers
     */
    private initWebSocketManagers() {
        this.websocketManagers = this.config.websocketURIs.map(uri => new WebSocketManager({ websocketURI: uri, appId: this.config.appId }));
    }

    /**
     * Initialize the storage
     */
    private initQueryStorage() {
        const storageConfig = { dbName: `topgun-${this.config.appId}`, storeName: 'queries' };
        this.queryStorage = this.config.storage
            ? new this.config.storage(storageConfig)
            : new IndexedDBStorage(storageConfig);
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
            this.websocketManagers.forEach(manager => {
                manager.connect();
            });
        }

        this.networkListener.listen((isOnline) => {
            if (isOnline === this.isOnline) {
                return;
            }
            this.isOnline = isOnline;
            this.websocketManagers.forEach(manager => {
                if (isOnline) {
                    manager.connect();
                } else {
                    manager.disconnect();
                }
            });
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

    /**
     * Add a callback to be called before the window is unloaded
     * @param cb The callback to call
     */
    public beforeUnload(cb: () => void) {
        this.beforeUnloadCbs.push(cb);
    }
}
