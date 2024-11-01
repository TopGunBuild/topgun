import { ClientConfig, NetworkListenerAdapter, QueryCb, QueryState } from "./types";
import { IndexedDBStorage } from "./storage/indexeddb-storage";
import { ConnectionState, WebSocketManager } from "./websocket";
import { WindowNetworkListener } from "./utils/window-network-listener";
import { compareArraysSimple, toHexString, windowOrGlobal } from "@topgunbuild/utils";
import { AbstractRequest, CancelSelectRequest, DataChangesRequest, IDataChangesRequest, Identifiable, ISelectResult, Payload, PutMessageRequest, RequestHeader, SelectRequest, SelectResultRequest } from "@topgunbuild/types";
import { MemoryStorage } from "./storage/memory-storage";
import { StorageManager } from "./storage/storage-manager";
import { transformSocketUrl } from "./utils/socket-url-transformer";
import WebSocket from "isomorphic-ws";
import { deserialize } from "@dao-xyz/borsh";
import { DataUtil, convertQueryToFilterTree } from '@topgunbuild/data-processing';
import { bigintTime } from "@topgunbuild/time";

/** Interface for items that can be stored */
export interface StoreItem extends Identifiable {
    $id: string;
    [key: string]: any;
}

/** Custom error types for better error handling */
export class StoreError extends Error {
    constructor(message: string, public readonly code: string) {
        super(message);
        this.name = 'StoreError';
    }
}

export class ConnectionError extends StoreError {
    constructor(message: string) {
        super(message, 'CONNECTION_ERROR');
    }
}

/**
 * The Store class is the main entry point for the TopGun client library.
 * It manages the connection to the websocket servers and the storage of data.
 */
export class Store {
    private websocketManager: WebSocketManager;
    private storageManager: StorageManager;
    private readonly config: ClientConfig;
    private networkListener: NetworkListenerAdapter;
    private beforeUnloadCbs: (() => void)[] = [];
    private queryCbs: Record<string, QueryState<any>> = {};
    
    private connectionState: ConnectionState = 'closed';
    private retryAttempts = 0;
    private readonly MAX_RETRY_ATTEMPTS = 3;
    private isOnline = false;

    /** The user ID associated with this store instance */
    public readonly userId: string;

    /** The team ID associated with this store instance */
    public readonly teamId: string;

    /**
     * Create a new Store
     * @param config The configuration for the store
     * @throws {StoreError} If required configuration is missing
     */
    constructor(config: ClientConfig) {
        if (!config.appId) {
            throw new StoreError('AppId is required', 'INVALID_CONFIG');
        }

        this.config = config;
        this.userId = 'config.userId';
        this.teamId = 'config.teamId';

        this.initWebSocketManagers();
        this.initStorageManager();
        this.initNetworkListener();
        this.initBeforeUnload();

        this.beforeUnloadCbs.push(() => this.disconnect());
    }

    /**
     * Add messages to the store
     * @param messages Array of messages to add
     * @throws {StoreError} If messages are invalid
     */
    public async addMessages<T extends StoreItem>(messages: T[]): Promise<void> {
        try {
            if (!Array.isArray(messages) || messages.length === 0) {
                throw new StoreError('Invalid messages array', 'INVALID_INPUT');
            }

            await this.upsert('message', messages);

            for (const message of messages) {
                const body = new PutMessageRequest({
                    channelId: 'test',
                    messageId: message.$id,
                    value: JSON.stringify(message)
                });
                await this.sendRequest(body);
            }
        } catch (error) {
            console.error('Failed to add messages:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add messages', 'ADD_MESSAGE_ERROR');
        }
    }

    /**
     * Send a request to the websocket
     * @param body The request body
     * @throws {ConnectionError} When not connected
     * @throws {StoreError} On request failure
     */
    public async sendRequest(body: AbstractRequest): Promise<void> {
        if (this.connectionState !== 'opened') {
            throw new ConnectionError('Cannot send request while disconnected');
        }

        const payload = new Payload({
            header: new RequestHeader({
                userId: this.userId,
                teamId: this.teamId,
                state: bigintTime()
            }),
            body
        });

        try {
            await this.storageManager.putPendingAction(payload);
            await this.websocketManager.send(payload.encode());
            await this.storageManager.deletePendingAction(payload.body.id);
        } catch (error) {
            console.error('Failed to send request:', error);
            throw new StoreError('Failed to send request', 'SEND_REQUEST_ERROR');
        }
    }

    /**
     * Update or insert data into storage
     * @param entity The entity type being stored
     * @param data The data to store
     * @throws {StoreError} If data is invalid
     */
    public async upsert<T extends StoreItem>(entity: string, data: T | T[]): Promise<void> {
        try {
            const items = Array.isArray(data) ? data : [data];

            if (items.some(item => !item.$id)) {
                throw new StoreError('All items must have an $id property', 'INVALID_INPUT');
            }

            await this.storageManager.upsert(entity, items);

            this.updateQueriesForEntity(entity, items);
        } catch (error) {
            console.error('Upsert failed:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to upsert data', 'UPSERT_ERROR');
        }
    }

    /**
     * Update queries affected by changes to an entity
     * @private
     */
    private updateQueriesForEntity<T extends StoreItem>(entity: string, items: T[]): void {
        Object.entries(this.queryCbs).forEach(([queryHash, state]) => {
            if (state.query.entity === entity && state.result) {
                const changes = DataUtil.processChanges(state.result.rows, items, state.filterCriteria);
                
                if (changes.length === 0) return;

                const changeRequest: IDataChangesRequest<StoreItem> = {
                    changes: changes.map(change => ({
                        element: change.item,
                        type: change.type,
                        timestamp: Date.now()
                    })),
                    collection: changes.map(c => c.item),
                    total: this.calculateNewTotal(state.result.total, changes),
                    queryHash
                };

                this.handleDataChanges(changeRequest);
            }
        });
    }

    /**
     * Calculate new total after changes
     * @private
     */
    private calculateNewTotal(currentTotal: number, changes: Array<{ type: string }>): number {
        const additions = changes.filter(c => c.type === 'added').length;
        const deletions = changes.filter(c => c.type === 'deleted').length;
        return currentTotal + additions - deletions;
    }

    /**
     * Handle data changes from websocket or local updates
     * @private
     */
    private handleDataChanges<T extends StoreItem>(messageBody: IDataChangesRequest<T>): void {
        const queryState = this.queryCbs[messageBody.queryHash];
        if (!queryState?.result) return;

        try {
            const updatedRows = this.processDataChanges(queryState.result.rows, messageBody);
            
            const updatedResult = {
                ...queryState.result,
                rows: messageBody.changes?.length
                    ? DataUtil.applySorting(updatedRows, { criteria: queryState.query.sort })
                    : updatedRows,
                total: messageBody.total
            };

            if (!compareArraysSimple(updatedResult.rows, queryState.result.rows)) {
                queryState.result = updatedResult;
                this.storageManager.saveQueryResult(messageBody.queryHash, updatedResult, queryState.query.entity);
                queryState.cbs.forEach(cb => cb(updatedResult));
            }
        } catch (error) {
            console.error('Failed to handle data changes:', error);
        }
    }

    /**
     * Process data changes and return updated rows
     * @private
     */
    private processDataChanges<T extends StoreItem>(
        currentRows: T[], 
        messageBody: IDataChangesRequest<T>
    ): T[] {
        let updatedRows = [...currentRows];

        if (messageBody.collection?.length) {
            return this.parseCollection(messageBody.collection);
        }

        if (messageBody.changes?.length) {
            for (const change of messageBody.changes) {
                try {
                    const parsedElement = this.parseElement(change.element) as T;
                    updatedRows = this.applyChange(updatedRows, parsedElement, change.type);
                } catch (error) {
                    console.error('Failed to process change:', error);
                }
            }
        }

        return updatedRows;
    }

    /**
     * Subscribe to a query
     * @param query The query to execute
     * @param cb The callback to call with the result
     * @returns Unsubscribe function
     * @throws {StoreError} If query is invalid
     */
    public subscribeQuery<T extends StoreItem>(
        query: SelectRequest, 
        cb: QueryCb<ISelectResult<T>>
    ): () => void {
        if (!query || !cb) {
            throw new StoreError('Query and callback are required', 'INVALID_INPUT');
        }

        try {
            const encodedQuery = query.encode();
            const queryHash = toHexString(encodedQuery);
            
            this.initializeQueryState(queryHash, query, cb);
            this.loadInitialQueryResult<T>(queryHash, query);

            return () => this.unsubscribeQuery(query, cb, queryHash);
        } catch (error) {
            console.error('Failed to subscribe to query:', error);
            throw new StoreError('Failed to subscribe to query', 'SUBSCRIBE_ERROR');
        }
    }

    /**
     * Initialize query state
     * @private
     */
    private initializeQueryState(
        queryHash: string, 
        query: SelectRequest, 
        cb: QueryCb<any>
    ): void {
        const queryState = this.queryCbs[queryHash];
        
        if (!queryState) {
            this.queryCbs[queryHash] = {
                query,
                cbs: [cb],
                result: null,
                filterCriteria: convertQueryToFilterTree(query)
            };
            this.websocketManager.send(query.encode());
        } else {
            queryState.cbs.push(cb);
        }
    }

    /**
     * Load initial query result from storage
     * @private
     */
    private async loadInitialQueryResult<T>(
        queryHash: string, 
        query: SelectRequest
    ): Promise<void> {
        try {
            const result = await this.storageManager.getQueryResult<T>(queryHash, query.entity);
            if (result) {
                const queryState = this.queryCbs[queryHash];
                queryState.result = result;
                queryState.cbs.forEach(cb => cb(result));
            }
        } catch (error) {
            console.error('Failed to load initial query result:', error);
        }
    }

    /**
     * Unsubscribe from a query
     * @throws {StoreError} If unsubscribe fails
     */
    public unsubscribeQuery(
        query: SelectRequest, 
        cb: QueryCb<any>, 
        queryHash?: string
    ): void {
        try {
            const hash = queryHash || toHexString(query.encode());
            const queryState = this.queryCbs[hash];
            
            if (!queryState) return;

            queryState.cbs = queryState.cbs.filter(q => q !== cb);

            if (queryState.cbs.length === 0) {
                this.cleanupQuery(hash, query);
            }
        } catch (error) {
            console.error('Failed to unsubscribe from query:', error);
            throw new StoreError('Failed to unsubscribe from query', 'UNSUBSCRIBE_ERROR');
        }
    }

    /**
     * Clean up query resources
     * @private
     */
    private async cleanupQuery(hash: string, query: SelectRequest): Promise<void> {
        delete this.queryCbs[hash];
        await this.storageManager.deleteQuery(hash, query.entity);

        const cancelRequest = new CancelSelectRequest({ queryHash: hash });
        await this.websocketManager.send(cancelRequest.encode());
    }

    /**
     * Handle websocket messages
     * @private
     */
    private handleWebSocketMessage(msg: WebSocket.Data): void {
        try {
            const message = deserialize(msg as Uint8Array, Payload);
            const messageBody = message.body;

            switch (true) {
                case messageBody instanceof SelectResultRequest:
                    this.handleSelectResult(messageBody);
                    break;
                case messageBody instanceof DataChangesRequest:
                    this.handleDataChanges<StoreItem>(messageBody as unknown as IDataChangesRequest<StoreItem>);
                    break;
                default:
                    console.warn('Unhandled message type:', messageBody);
            }
        } catch (error) {
            console.error('Failed to handle websocket message:', error);
        }
    }

    /**
     * Initialize websocket connection
     * @private
     */
    private initWebSocketManagers(): void {
        this.websocketManager = new WebSocketManager({
            websocketURI: transformSocketUrl(this.config.websocketURI),
            appId: this.config.appId,
            onStateChange: this.handleConnectionStateChange.bind(this)
        });
        this.websocketManager.addMessageHandler(this.handleWebSocketMessage.bind(this));
    }

    /**
     * Handle connection state changes
     * @private
     */
    private handleConnectionStateChange(state: ConnectionState): void {
        this.connectionState = state;
        if (state === 'closed' && this.isOnline) {
            this.retryConnection();
        }
    }

    /**
     * Retry connection with exponential backoff
     * @private
     */
    private async retryConnection(): Promise<void> {
        if (this.retryAttempts >= this.MAX_RETRY_ATTEMPTS) {
            console.error('Max retry attempts reached');
            return;
        }

        const delay = Math.min(1000 * Math.pow(2, this.retryAttempts), 30000);
        this.retryAttempts++;

        await new Promise(resolve => setTimeout(resolve, delay));
        this.websocketManager.connect();
    }

    /**
     * Disconnect from the websocket
     * @public
     */
    public disconnect(): void {
        this.connectionState = 'closed';
        this.websocketManager.disconnect();
        this.retryAttempts = 0;
    }

    /**
     * Clean up resources
     * @public
     */
    public destroy(): void {
        this.disconnect();
        this.beforeUnloadCbs = [];
        this.queryCbs = {};
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

    /**
     * Parse collection data
     * @private
     */
    private parseCollection<T>(collection: any[]): T[] {
        try {
            return collection.map(item =>
                typeof item === 'string' ? JSON.parse(item) : item
            );
        } catch (error) {
            console.error('Failed to parse collection:', error);
            throw new StoreError('Invalid collection data', 'PARSE_ERROR');
        }
    }

    /**
     * Parse element data
     * @private
     */
    private parseElement<T extends StoreItem>(element: any): T {
        const parsed = typeof element === 'string' ? JSON.parse(element) : element;
        if (!parsed.$id) {
            throw new StoreError('Invalid element: missing $id', 'PARSE_ERROR');
        }
        return parsed;
    }

    /**
     * Apply a single change to the rows
     * @private
     */
    private applyChange<T extends StoreItem>(
        rows: T[], 
        element: T, 
        changeType: 'added' | 'updated' | 'deleted'
    ): T[] {
        switch (changeType) {
            case 'deleted':
                return rows.filter(row => row.$id !== element.$id);
            case 'added':
                return [...rows, element];
            case 'updated':
                const index = rows.findIndex(row => row.$id === element.$id);
                if (index !== -1) {
                    return [
                        ...rows.slice(0, index),
                        element,
                        ...rows.slice(index + 1)
                    ];
                }
                return rows;
            default:
                return rows;
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
}