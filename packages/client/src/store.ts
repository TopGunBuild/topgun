import { ClientConfig, NetworkListenerAdapter, QueryCb, QueryState } from "./types";
import { IndexedDBStorage } from "./storage/indexeddb-storage";
import { WebSocketManager } from "./websocket";
import { WindowNetworkListener } from "./utils/window-network-listener";
import { compareArraysSimple, toHexString, windowOrGlobal } from "@topgunbuild/utils";
import { CancelSelectRequest, DataChangesRequest, IDataChangesRequest, Identifiable, ISelectResult, Payload, SelectRequest, SelectResultRequest } from "@topgunbuild/types";
import { MemoryStorage } from "./storage/memory-storage";
import { StorageManager } from "./storage/storage-manager";
import { transformSocketUrl } from "./utils/socket-url-transformer";
import WebSocket from "isomorphic-ws";
import { deserialize } from "@dao-xyz/borsh";
import { DataUtil, convertQueryToFilterTree } from '@topgunbuild/data-processing';

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

        this.beforeUnload(() => this.disconnect());
    }

    /**
     * Update or insert data into storage
     * @param entity The entity type being stored
     * @param data The data to store
     */
    public async upsert<T extends Identifiable>(entity: string, data: T | T[]): Promise<void> {
        const items = Array.isArray(data) ? data : [data];
        
        // Validate all items have IDs
        if (items.some(item => !item.$id)) {
            throw new Error('All items must have an $id property');
        }

        // Store in local storage
        await this.storageManager.upsert(entity, items);

        // Update any active queries that may be affected
        Object.entries(this.queryCbs).forEach(([queryHash, state]) => {
            if (state.query.entity === entity && state.result) {

                // Process changes through data util
                const changes = DataUtil.processChanges(state.result.rows, items, state.filterCriteria);
                
                // Create change notification
                const changeRequest: IDataChangesRequest<Identifiable> = {
                    changes: changes.map(change => ({
                        element: change.item,
                        type: change.type,
                        timestamp: Date.now()
                    })),
                    collection: changes.map(c => c.item),
                    total: state.result.total + (changes.filter(c => c.type === 'added').length - changes.filter(c => c.type === 'deleted').length),
                    queryHash
                };

                this.handleDataChanges(changeRequest);
            }
        });
    }

    /**
     * Subscribe to a query
     * @param query The query to execute
     * @param cb The callback to call with the result
     */
    public subscribeQuery<T extends Identifiable>(query: SelectRequest, cb: QueryCb<ISelectResult<T>>): () => void {
        // Encode and hash query once, store in const
        const encodedQuery = query.encode();
        const queryHash = toHexString(encodedQuery);
        const queryState = this.queryCbs[queryHash];

        // Use existing queryState if available
        if (!queryState) {
            this.queryCbs[queryHash] = {
                query,
                cbs: [cb], // Initialize with the callback directly
                result: null,
                filterCriteria: convertQueryToFilterTree(query)
            };
            this.websocketManager.send(encodedQuery);
        } else {
            queryState.cbs.push(cb);
        }

        // Load result from storage if available
        this.storageManager.getQueryResult<T>(queryHash, query.entity).then(result => {
            if (result) {
                queryState.result = result;
                queryState.cbs.forEach(cb => cb(result));
            }
        });

        // Return memoized unsubscribe function
        return () => this.unsubscribeQuery(query, cb, queryHash);
    }

    /**
     * Unsubscribe from a query
     * @param query The query to unsubscribe from
     * @param cb The callback to unsubscribe
     * @param queryHash The hash of the query
     */
    public unsubscribeQuery(query: SelectRequest, cb: QueryCb<any>, queryHash?: string) {
        // Get or compute queryHash
        const hash = queryHash || toHexString(query.encode());
        
        // Early return if query doesn't exist
        const queryState = this.queryCbs[hash];
        if (!queryState) {
            return;
        }

        // Filter callbacks
        queryState.cbs = queryState.cbs.filter(q => q !== cb);
        
        // Clean up if no callbacks remain
        if (queryState.cbs.length === 0) {
            delete this.queryCbs[hash];
            this.storageManager.deleteQuery(hash, query.entity);

            // Cancel the query on the server
            const cancelRequest = new CancelSelectRequest({ queryHash: hash });
            this.websocketManager.send(cancelRequest.encode());
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
     * Disconnect from the websocket
     */
    public disconnect() {
        this.websocketManager.disconnect();
    }

    /**
     * Initialize the websocket managers
     */
    private initWebSocketManagers() {
        this.websocketManager = new WebSocketManager({
            websocketURI: transformSocketUrl(this.config.websocketURI),
            appId: this.config.appId
        });
        this.websocketManager.addMessageHandler(msg => this.handleWebSocketMessage(msg));
    }

    /**
     * Handle a message from the websocket
     * @param msg The message to handle
     */
    private handleWebSocketMessage(msg: WebSocket.Data) {
        const message = deserialize(msg as Uint8Array, Payload);
        const messageBody = message.body;

        switch (true) {
            case messageBody instanceof SelectResultRequest:
                this.handleSelectResult(messageBody);
                break;

            case messageBody instanceof DataChangesRequest:
                this.handleDataChanges<string>(messageBody);
                break;
        }
    }

    /**
     * Handle data changes
     * @param messageBody The data changes request
     */
    private handleDataChanges<T>(messageBody: IDataChangesRequest<T>) {
        const queryHash = messageBody.queryHash;
        const queryState = this.queryCbs[queryHash];

        if (!queryState || !queryState.result) {
            return;
        }

        let updatedRows = [...queryState.result.rows];

        // Handle full collection replacement
        if (messageBody.collection?.length) {
            try {
                const parsedCollection = messageBody.collection.map(item => 
                    typeof item === 'string' ? JSON.parse(item) : item
                );
                updatedRows = parsedCollection;
            } catch (e) {
                console.error('Failed to parse collection data:', e);
                return;
            }
        }
        // Handle individual changes
        else if (messageBody.changes?.length) {
            for (const change of messageBody.changes) {
                try {
                    const parsedElement = typeof change.element === 'string' 
                        ? JSON.parse(change.element) 
                        : change.element;
                    
                    if (change.type === 'deleted') {
                        updatedRows = updatedRows.filter(row => row.id !== parsedElement.id);
                    } else if (change.type === 'added') {
                        updatedRows = [...updatedRows, parsedElement];
                    }
                } catch (e) {
                    console.error('Failed to parse change element:', e);
                    continue;
                }
            }
        } else {
            return; // No changes to apply
        }

        // Create updated result with sorted rows
        const updatedResult = {
            ...queryState.result,
            rows: messageBody.changes?.length 
                ? DataUtil.applySorting(updatedRows, { criteria: queryState.query.sort })
                : updatedRows,
            total: messageBody.total
        };

        if (!compareArraysSimple(updatedResult.rows, queryState.result.rows)) {
            // Update query state and notify
            queryState.result = updatedResult;
            this.storageManager.saveQueryResult(queryHash, updatedResult, queryState.query.entity);
            queryState.cbs.forEach(cb => cb(updatedResult));
        }
    }

    /**
     * Handle a select result
     * @param action The select result to handle
     */
    private handleSelectResult(query: ISelectResult<any>): void {
        const queryHash = query.queryHash;
        const queryState = this.queryCbs[queryHash];

        if (!queryState) {
            return;
        }

        queryState.result = query;
        queryState.cbs.forEach(cb => cb(query));

        this.storageManager.saveQueryResult(queryHash, query, queryState.query.entity);
    }   

    /**
     * Initialize the storage manager
     */
    private initStorageManager() {
        const dbName = `topgun-${this.config.appId}`;
        const storageImpl = this.config.storage || 
            (IndexedDBStorage.isSupported() ? IndexedDBStorage : MemoryStorage);
            
        this.storageManager = new StorageManager(dbName, storageImpl);
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