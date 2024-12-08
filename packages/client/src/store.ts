import { ClientConfig, NetworkListenerAdapter, QueryCb, QueryState } from "./types";
import { IndexedDBStorage } from "./storage/indexeddb-storage";
import { WindowNetworkListener } from "./utils/window-network-listener";
import { compareArraysSimple, randomId, toHexString, windowOrGlobal } from "@topgunbuild/common";
import { MemoryStorage } from "./storage/memory-storage";
import { StorageManager } from "./storage/storage-manager";
import { transformSocketUrl } from "./utils/socket-url-transformer";
import {
    TransportPayloadImpl,
    DataChangesAction,
    SelectAction,
    CancelSelectAction,
    SelectResultAction,
    DataChanges,
    Identifiable,
    LocalContext,
    SelectResult,
    AbstractAction,
    UserWithSecrets,
    DeviceWithSecrets,
    EncryptedPayloadImpl,
    StoreItem,
    deserialize
} from "@topgunbuild/models";
import { encryptPayload } from "@topgunbuild/model-utils";
import { ChangeType, DataUtil } from "@topgunbuild/collections";
import { convertQueryToFilterGroup } from "@topgunbuild/frames";
import { WebSocketConnector } from "./websocket-connector";
import { ConnectorState } from "@topgunbuild/control-flow";
import { asymmetric } from "@topgunbuild/crypto";
import { LoggerService } from "@topgunbuild/logger";
import { StoreError } from "./errors";
import { whereString } from "./query-conditions";

/**
 * The Store class is the main entry point for the TopGun client library.
 * It manages the connection to the websocket servers and the storage of data.
 */
export class Store {
    private connector: WebSocketConnector;
    private storageManager: StorageManager;
    private networkListener: NetworkListenerAdapter;
    private beforeUnloadCbs: (() => void)[] = [];
    private activeQueries: Record<string, QueryState<any>> = {};

    private connectionState: ConnectorState = 'closed';
    private retryAttempts = 0;
    private readonly MAX_RETRY_ATTEMPTS = 3;
    private isOnline = false;
    public context: LocalContext;
    private readonly remoteQueryTimeout: number;

    /**
     * Create a new Store
     * @param config The configuration for the store
     * @throws {StoreError} If required configuration is missing
     */
    constructor(
        public readonly config: ClientConfig,
        private readonly logger: LoggerService,
    ) {
        if (!config.appId) {
            throw new StoreError('AppId is required', 'INVALID_CONFIG');
        }
        // if (!this.user?.keys?.encryption?.secretKey) {
        //     throw new StoreError('User encryption keys are required', 'INVALID_CONFIG');
        // }

        this.remoteQueryTimeout = config.remoteQueryTimeout || 5000;

        this.initWebSocketConnector();
        this.initStorageManager();
        this.initNetworkListener();
        this.initBeforeUnload();

        this.beforeUnloadCbs.push(() => this.disconnect());
    }

    get isServer(): boolean {
        return 'server' in this.context
    }

    /**
     * Get the user
     * @returns The user
     */
    async getUser(): Promise<UserWithSecrets | null>
    {
        return null;
    }

    /**
     * Get the device
     * @returns The device
     */
    async getDevice(): Promise<DeviceWithSecrets | null>
    {
        return null;
    }

    /**
     * Subscribe to a query
     * @param query The query to execute
     * @param cb The callback to call with the result
     * @returns Unsubscribe function
     * @throws {StoreError} If query is invalid
     */
    public subscribeQuery<T extends StoreItem>(
        query: SelectAction,
        cb: QueryCb<SelectResult<T>>
    ): () => void {
        if (!query || !cb) {
            throw new StoreError('Query and callback are required', 'INVALID_INPUT');
        }

        try {
            const encodedQuery = query.encode();
            const queryHash = toHexString(encodedQuery);

            this.initializeQueryState(queryHash, query, cb);
            this.dispatchAction(query);
            this.initializeQueryResult(queryHash, query);

            return () => this.unsubscribeQuery(query, cb, queryHash);
        } catch (error) {
            this.logger.error('Failed to subscribe to query:', error);
            throw new StoreError('Failed to subscribe to query', 'SUBSCRIBE_ERROR');
        }
    }

    /**
     * Unsubscribe from a query
     * @throws {StoreError} If unsubscribe fails
     */
    public unsubscribeQuery(
        query: SelectAction,
        cb: QueryCb<any>,
        queryHash?: string
    ): void {
        try {
            const hash = queryHash || toHexString(query.encode());
            const queryState = this.activeQueries[hash];

            if (!queryState) return;

            queryState.cbs = queryState.cbs.filter(q => q !== cb);

            if (!this.isQueryActive(hash)) {
                this.cleanupQuery(hash, query);
            }
        } catch (error) {
            this.logger.error('Failed to unsubscribe from query:', error);
            throw new StoreError('Failed to unsubscribe from query', 'UNSUBSCRIBE_ERROR');
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
            this.logger.error('Upsert failed:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to upsert data', 'UPSERT_ERROR');
        }
    }

    /**
     * Delete items from storage
     * @param entity The entity type
     * @param ids Array of item IDs to delete
     * @throws {StoreError} If deletion fails
     */
    public async delete(entity: string, ids: string[]): Promise<void> {
        try {
            if (!Array.isArray(ids) || ids.length === 0) {
                throw new StoreError('Invalid ids array', 'INVALID_INPUT');
            }

            await this.storageManager.delete(entity, ids);

            // Create dummy items for query updates
            const deletedItems = ids.map(id => ({ $id: id }));
            this.updateQueriesForEntity(entity, deletedItems);

        } catch (error) {
            this.logger.error('Delete failed:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to delete data', 'DELETE_ERROR');
        }
    }

    /**
     * Send an action to the websocket
     * @param body The action body
     * @throws {StoreError} On action failure
     */
    public async dispatchAction(body: AbstractAction): Promise<void> {
        if (!body) {
            throw new StoreError('Action body is required', 'INVALID_INPUT');
        }

        try {
            const payload = await this.createEncryptedPayload(body);
            this.sendOrQueuePayload(payload, body);
        } catch (error) {
            this.logger.error('Failed to send action:', error);
            throw error instanceof StoreError
                ? error
                : new StoreError('Failed to send action', 'SEND_ACTION_ERROR');
        }
    }

    /**
     * Disconnect from the websocket
     * @public
     */
    public disconnect(): void {
        this.connectionState = 'closed';
        this.connector.disconnect();
        this.retryAttempts = 0;
    }

    /**
     * Clean up resources
     * @public
     */
    public destroy(): void {
        this.disconnect();
        this.beforeUnloadCbs = [];
        this.activeQueries = {};
    }

    /**
     * Check if an item exists in storage
     * @param entity The entity type to check
     * @param id The item ID to check
     * @returns Promise<boolean> True if the item exists, false otherwise
     */
    public async exists(entity: string, id: string): Promise<boolean> {
        try {
            const result = await this.getById(entity, id);
            return result.data !== null;
        } catch (error) {
            return false;
        }
    }

    /**
     * Get an item by ID from local or remote storage
     * @param entity The entity type to query
     * @param id The item ID to retrieve
     * @returns Promise<T> The item
     * @throws {StoreError} If retrieval fails
     */
    public async getById<T extends StoreItem>(entity: string, id: string): Promise<T> {
        if (!entity || !id) {
            throw new StoreError('Entity and ID are required', 'INVALID_INPUT');
        }

        try {
            // Check local storage first
            const localItem = await this.storageManager.get<T>(entity, id);
            if (localItem) {
                return localItem;
            }

            // If not in local storage, query remote
            const query = new SelectAction({
                entity,
                query: [whereString('$id', '=', id)],
                pageSize: 1
            });

            const remoteItems = await this.getRemoteItems<T>(query);
            const remoteItem = remoteItems[0] || null;
            
            // If found remotely, store it locally for future use
            if (remoteItem) {
                await this.storageManager.upsert(entity, [remoteItem]);
            }

            return remoteItem;
        } catch (error) {
            this.logger.error('Failed to get item by ID:', error);
            throw new StoreError(`Failed to get ${entity} with ID ${id}`, 'GET_BY_ID_ERROR');
        }
    }

    /**
     * Get multiple items by SelectAction from local or remote storage
     * @param entity The entity type to query
     * @param selectAction The SelectAction containing the query criteria
     * @returns Promise<T[]> The items
     * @throws {StoreError} If retrieval fails
     */
    public async query<T extends StoreItem>(entity: string, selectAction: SelectAction): Promise<T[]> {
        if (!entity || !selectAction) {
            throw new StoreError('Entity and SelectAction are required', 'INVALID_INPUT');
        }

        try {
            // Check local storage first
            const localItems = await this.storageManager.query<T>(entity, selectAction);
            
            // If all items found locally, return them
            if (localItems.length === selectAction.pageSize) {
                return localItems;
            }

            // Query remote for items
            const remoteItems = await this.getRemoteItems<T>(selectAction);
            
            // Store remote items locally for future use
            if (remoteItems.length > 0) {
                await this.storageManager.upsert(entity, remoteItems);
            }

            return remoteItems;
        } catch (error) {
            this.logger.error('Failed to get items by SelectAction:', error);
            throw new StoreError(`Failed to get ${entity} with provided SelectAction`, 'GET_BY_IDS_ERROR');
        }
    }

    /**
     * Create an encrypted payload
     * @param action The action to encrypt
     * @returns The encrypted payload
     */
    private async createEncryptedPayload(action: AbstractAction): Promise<Uint8Array> {
        return encryptPayload({
            user: await this.getUser(),
            recipientPublicKey: null,
            action
        });
    }

    /**
     * Send or queue a payload
     * @param payload The payload to send
     * @param action The action to send
     */
    private sendOrQueuePayload(payload: Uint8Array, action: AbstractAction): void {
        if (this.connector.isConnected) {
            this.connector.send(payload);
        } else if (!(action instanceof SelectAction)) {
            this.storageManager.putPendingAction(randomId(), payload);
        }
    }

    /**
     * Update queries affected by changes to an entity
     * @private
     */
    private updateQueriesForEntity<T extends StoreItem>(entity: string, items: T[]): void {
        Object.entries(this.activeQueries).forEach(([queryHash, state]) => {
            if (state.query.entity === entity && state.result) {
                const changes = DataUtil.processChanges(state.result.rows, items, state.filterOptions);

                if (changes.length === 0) return;

                const changeRequest: DataChanges<StoreItem> = {
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
     * Initialize query state
     * @private
     */
    private initializeQueryState(
        queryHash: string,
        query: SelectAction,
        cb: QueryCb<any>
    ): void {
        const queryState = this.activeQueries[queryHash];

        if (!queryState) {
            this.activeQueries[queryHash] = {
                query,
                cbs: [cb],
                result: null,
                filterOptions: convertQueryToFilterGroup(query)
            };
        } else {
            queryState.cbs.push(cb);
        }
    }

    /**
     * Load cached query result and notify subscribers
     * @private
     */
    private async initializeQueryResult(
        queryHash: string,
        query: SelectAction
    ): Promise<void> {
        try {
            const queryState = this.activeQueries[queryHash];
            if (!queryState) {
                throw new StoreError('Query state not found', 'INVALID_STATE');
            }

            // Try to get cached result first
            const cachedResult = await this.storageManager.getQueryResult<Identifiable>(
                queryHash,
                query.entity
            );

            if (cachedResult) {
                queryState.result = cachedResult;
                for (const cb of queryState.cbs) {
                    try {
                        cb(cachedResult);
                    } catch (cbError) {
                        this.logger.error('Callback error:', cbError);
                    }
                }
                return;
            }

            // If no cached result, process all items
            const allItems = await this.storageManager.getAll(query.entity);
            const processedResult = DataUtil.processDataset(allItems, {
                filter: { options: queryState.filterOptions },
                sort: { options: query.sort },
                page: {
                    currentPage: query.pageOffset,
                    itemsPerPage: query.pageSize,
                },
            });

            // Always cache the result, even if empty
            const newResult = {
                rows: processedResult.rows,
                total: processedResult.total
            };

            queryState.result = newResult;
            this.storageManager.saveQueryResult(queryHash, newResult, query.entity);

            // Notify subscribers
            for (const cb of queryState.cbs) {
                try {
                    cb(newResult);
                } catch (cbError) {
                    this.logger.error('Callback error:', cbError);
                }
            }
        } catch (error) {
            this.logger.error('Failed to load initial query result:', error);
            throw new StoreError('Failed to load query result', 'QUERY_LOAD_ERROR');
        }
    }

    /**
     * Clean up query resources
     * @private
     */
    private async cleanupQuery(hash: string, query: SelectAction): Promise<void> {
        delete this.activeQueries[hash];
        await this.storageManager.deleteQuery(hash, query.entity);

        const cancelRequest = new CancelSelectAction({ queryHash: hash });
        this.dispatchAction(cancelRequest);
    }

    /**
     * Initialize the websocket connector
     * @private
     */
    private initWebSocketConnector(): void {
        this.connector = new WebSocketConnector({
            websocketURI: transformSocketUrl(this.config.websocketURI),
            appId: this.config.appId
        });
        this.connector.useInputMiddleware(async (msg: Uint8Array) => {
            try {
                const payload = deserialize(msg, EncryptedPayloadImpl);
                if (!payload) {
                    throw new StoreError('Failed to deserialize encrypted payload', 'DESERIALIZE_ERROR');
                }

                const decryptedPayload = asymmetric.decryptBytes({
                    cipher: payload.encryptedBody,
                    recipientSecretKey: (await this.getUser())?.keys?.encryption?.secretKey,
                    senderPublicKey: payload.senderPublicKey
                });
                if (!decryptedPayload) {
                    throw new StoreError('Failed to decrypt payload', 'DECRYPT_ERROR');
                }

                return deserialize(decryptedPayload, TransportPayloadImpl);
            } catch (error) {
                this.logger.error('Input middleware error:', error);
                throw error instanceof StoreError ? error : new StoreError('Input middleware failed', 'MIDDLEWARE_ERROR');
            }
        });
        this.connector.on('receiveMessage', (msg: TransportPayloadImpl) => {
            this.handleIncomingMessage(msg);
        });
        this.connector.on('stateChange', (state: ConnectorState) => {
            this.handleConnectionStateChange(state);
        });
    }

    /**
     * Handle incoming messages
     * @private
     */
    private handleIncomingMessage(message: TransportPayloadImpl): void {
        try {
            if (!message?.body) {
                throw new StoreError('Invalid message received', 'INVALID_MESSAGE');
            }
            const messageBody = message.body;

            switch (true) {
                case messageBody instanceof SelectResultAction:
                    this.handleSelectResult(messageBody);
                    break;
                case messageBody instanceof DataChangesAction:
                    this.handleDataChanges<StoreItem>(messageBody as unknown as DataChanges<StoreItem>);
                    break;
                default:
                    throw new StoreError(`Unhandled message type: ${messageBody.constructor.name}`, 'UNKNOWN_MESSAGE_TYPE');
            }
        } catch (error) {
            this.logger.error('Failed to handle websocket message:', error);
            throw error instanceof StoreError ? error : new StoreError('Message handling failed', 'MESSAGE_HANDLING_ERROR');
        }
    }

    /**
    * Handle a select result
    * @param action The select result to handle
    */
    private handleSelectResult(query: SelectResult<any>): void {
        const queryHash = query.queryHash;
        const queryState = this.activeQueries[queryHash];

        if (!queryState) {
            return;
        }

        queryState.result = query;
        queryState.cbs.forEach(cb => cb(query));

        this.storageManager.saveQueryResult(queryHash, query, queryState.query.entity);
    }

    /**
     * Handle data changes from websocket or local updates
     * @private
     */
    private handleDataChanges<T extends StoreItem>(messageBody: DataChanges<T>): void {
        const queryState = this.activeQueries[messageBody.queryHash];
        if (!queryState?.result) return;

        try {
            const updatedRows = this.processDataChanges(queryState.result.rows, messageBody);

            const updatedResult = {
                ...queryState.result,
                rows: messageBody.changes?.length
                    ? DataUtil.applySorting(updatedRows, { options: queryState.query.sort })
                    : updatedRows,
                total: messageBody.total
            };

            if (!compareArraysSimple(updatedResult.rows, queryState.result.rows)) {
                queryState.result = updatedResult;
                this.storageManager.saveQueryResult(messageBody.queryHash, updatedResult, queryState.query.entity);
                queryState.cbs.forEach(cb => cb(updatedResult));
            }
        } catch (error) {
            this.logger.error('Failed to handle data changes:', error);
        }
    }

    /**
     * Process data changes and return updated rows
     * @private
     */
    private processDataChanges<T extends StoreItem>(
        currentRows: T[],
        messageBody: DataChanges<T>
    ): T[] {
        let updatedRows = [...currentRows];

        if (messageBody.collection?.length) {
            return this.parseCollection(messageBody.collection);
        }

        if (messageBody.changes?.length) {
            for (const change of messageBody.changes) {
                try {
                    const parsedElement = this.parseElement(change.element) as T;
                    updatedRows = this.applyChange(updatedRows, parsedElement, change.type as ChangeType);
                } catch (error) {
                    this.logger.error('Failed to process change:', error);
                }
            }
        }

        return updatedRows;
    }

    /**
     * Apply a single change to the rows
     * @private
     */
    private applyChange<T extends StoreItem>(
        rows: T[],
        element: T,
        changeType: ChangeType
    ): T[] {
        switch (changeType) {
            case ChangeType.Deleted:
                return rows.filter(row => row.$id !== element.$id);
            case ChangeType.Added:
                return [...rows, element];
            case ChangeType.Updated:
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
     * Handle connection state changes
     * @private
     */
    private handleConnectionStateChange(state: ConnectorState): void {
        const previousState = this.connectionState;
        this.connectionState = state;

        if (state === 'opened') {
            this.retryAttempts = 0;
            // Add delay to ensure connection is stable
            setTimeout(() => {
                this.resubscribeQueries().catch(error => {
                    this.logger.error('Failed to resubscribe after connection:', error);
                });
            }, 100);
        } else if (state === 'closed' && this.isOnline && previousState === 'opened') {
            this.retryConnection();
        }
    }

    /**
     * Retry connection with exponential backoff
     * @private
     */
    private async retryConnection(): Promise<void> {
        if (this.retryAttempts >= this.MAX_RETRY_ATTEMPTS) {
            this.logger.error('Max retry attempts reached');
            return;
        }

        const delay = Math.min(1000 * Math.pow(2, this.retryAttempts), 30000);
        this.retryAttempts++;

        await new Promise(resolve => setTimeout(resolve, delay));
        this.connector.startSocket();
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
            this.connector.startSocket();
        }

        this.networkListener.listen((isOnline) => {
            if (isOnline === this.isOnline) {
                return;
            }
            this.isOnline = isOnline;
            if (isOnline) {
                this.connector.startSocket();
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
            this.logger.error('Failed to parse collection:', error);
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
     * Resubscribe to all active queries after reconnection
     * @private
     */
    private async resubscribeQueries(): Promise<void> {
        try {
            // Get pending actions first
            const pendingActions = await this.storageManager.getAllPendingActions();

            // Process pending actions
            for (const [id, action] of Object.entries(pendingActions)) {
                try {
                    this.connector.send(action);
                    this.storageManager.deletePendingAction(id);
                } catch (error) {
                    this.logger.error(`Failed to process pending action ${id}:`, error);
                }
            }

            // Resubscribe to active queries
            const activeQueries = Object.entries(this.activeQueries);
            if (activeQueries.length === 0) return;

            this.logger.verbose(`Resubscribing to ${activeQueries.length} queries...`);

            for (const [queryHash, state] of activeQueries) {
                try {
                    // Dispatch the resubscription
                    await this.dispatchAction(state.query);

                    // Notify subscribers with cached data while waiting for fresh data
                    if (state.result) {
                        state.cbs.forEach(cb => cb(state.result));
                    }
                } catch (error) {
                    this.logger.error(`Failed to resubscribe to query ${queryHash}:`, error);
                    // Continue with other queries even if one fails
                }
            }
        } catch (error) {
            this.logger.error('Failed to resubscribe queries:', error);
            throw new StoreError('Query resubscription failed', 'RESUBSCRIBE_ERROR');
        }
    }

    /**
     * Check if a query is still active
     * @private
     */
    private isQueryActive(queryHash: string): boolean {
        const queryState = this.activeQueries[queryHash];
        return queryState != null && queryState.cbs.length > 0;
    }

    /**
     * Get items from remote storage
     * @private
     */
    private getRemoteItems<T>(query: SelectAction): Promise<T[]> {
        return new Promise((resolve) => {
            const timeout = setTimeout(() => resolve([]), this.remoteQueryTimeout);

            const unsubscribe = this.subscribeQuery(query, (result) => {
                clearTimeout(timeout);
                unsubscribe();
                resolve(result.rows as T[]);
            });
        });
    }
}
