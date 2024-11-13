import { Identifiable, Password, SelectResult } from "@topgunbuild/models";
import { textEncoder } from '@topgunbuild/textencoder';
import { PersistedService } from "./persisted-service";
import { StorageDerived } from "./types";

// Add these at the top with other types
interface StorageServiceParams {
    dbName: string;
    storeName: string;
    encryptionKey?: Password;
}

interface StoredQueryResult<T> {
    rows: T[];
    total: number;
    hasNextPage: boolean;
    hasPreviousPage: boolean;
}

type StorageRetryOptions = {
    maxRetries?: number;
    delayMs?: number;
};

/**
 * The storage manager
 */
export class StorageManager {
    private readonly entityDataStorages: Map<string, {
        storage: PersistedService<Uint8Array>;
        lastAccessed: number;
    }> = new Map();
    private readonly queryStorage: PersistedService<Uint8Array>;
    private readonly pendingActionsStorage: PersistedService<Uint8Array>;
    private readonly storage: StorageDerived<any>;
    private readonly dbName: string;
    private readonly storagePassphrase?: Password;

    /**
     * Constructor
     * @param dbName - The name of the database
     * @param storage - The storage adapter
     */
    constructor(
        dbName: string,
        storage: StorageDerived<any>,
        storagePassphrase?: Password
    ) {
        this.dbName = dbName;
        this.storage = storage;
        this.storagePassphrase = storagePassphrase;

        // Initialize common storages using the new method
        this.queryStorage = this.initializeStorage('queries');
        this.pendingActionsStorage = this.initializeStorage('pendingActions');
    }

    /**
     * Get the entity storage
     * @param entity - The entity
     * @returns The entity storage
     */
    private getEntityStorage(entity: string): PersistedService<Uint8Array> {
        const now = Date.now();
        const existing = this.entityDataStorages.get(entity);
        
        if (existing) {
            existing.lastAccessed = now;
            return existing.storage;
        }

        const storage = new PersistedService({
            params: { 
                dbName: this.dbName, 
                storeName: `data_${entity}`, 
                encryptionKey: this.storagePassphrase 
            },
            storage: this.storage,
            merge: (fromStorage, currentValue) => {
                if (!fromStorage || !currentValue) {
                    return fromStorage || currentValue || {};
                }
                return { ...fromStorage, ...currentValue };
            }
        });

        this.entityDataStorages.set(entity, { storage, lastAccessed: now });
        return storage;
    }

    /**
     * Get all pending actions from storage
     * @returns An array of pending actions
     */
    public async getAllPendingActions(): Promise<Uint8Array[]> {
        await this.pendingActionsStorage.waitForLoaded();
        return Object.values(this.pendingActionsStorage.value);
    }

    /**
     * Put a pending action into the storage
     * @param action - The action
     */
    public putPendingAction(id: string, action: Uint8Array) {
        this.pendingActionsStorage.set(id, action);
    }

    /**
     * Get a pending action from the storage
     * @param id - The id of the action
     */
    public async getPendingAction(id: string): Promise<Uint8Array | undefined> {
        await this.pendingActionsStorage.waitForLoaded();
        return this.pendingActionsStorage.get(id);
    }

    /**
     * Delete a pending action from the storage
     * @param id - The id of the action
     */
    public deletePendingAction(id: string) {
        this.pendingActionsStorage.delete(id);
    }

    /**
     * Delete items from entity storage
     * @param entity - The entity type 
     * @param ids - Array of item IDs to delete
     */
    public async delete(entity: string, ids: string[]): Promise<void> {
        const entityStorage = this.getEntityStorage(entity);
        await entityStorage.waitForLoaded();

        for (const id of ids) {
            entityStorage.delete(id);
        }
    }

    /**
     * Update or insert data into entity storage
     * @param entity - The entity type
     * @param data - The data to store
     */
    public async upsert<T extends Identifiable>(entity: string, data: T[]): Promise<void> {
        const entityStorage = this.getEntityStorage(entity);
        await this.ensureStoragesLoaded(entityStorage);
        
        await Promise.all(
            data.map(async item => {
                const encoded = await this.encodeItem(item, item.$id);
                entityStorage.set(item.$id, encoded);
            })
        );
    }

    /**
     * Get an item from entity storage
     * @param entity - The entity type
     * @param id - The id of the item
     * @returns The item
     */
    public async get<T extends Identifiable>(entity: string, id: string): Promise<T | undefined> {
        const entityStorage = this.getEntityStorage(entity);
        await entityStorage.waitForLoaded();
        
        const data = entityStorage.get(id);
        if (!data) return undefined;
        
        try {
            return await this.decodeItem<T>(data, id);
        } catch (error) {
            throw new Error(`Failed to decode item ${id}: ${error['message']}`);
        }
    }

    /**
     * Put a query into the storage
     * @param queryHash - The hash of the query
     * @param query - The query result
     * @param entity - The entity for the query
     */
    public async saveQueryResult<T extends Identifiable>(
        queryHash: string, 
        query: SelectResult<T>, 
        entity: string
    ): Promise<void> {
        const queryResult: StoredQueryResult<string> = {
            rows: query.rows.map(row => row.$id),
            total: query.total,
            hasNextPage: query.hasNextPage,
            hasPreviousPage: query.hasPreviousPage,
        };

        const entityStorage = this.getEntityStorage(entity);
        
        await Promise.all([
            this.withRetry(async () => {
                const encoded = await this.encodeItem(queryResult, queryHash);
                this.queryStorage.set(queryHash, encoded);
            }),
            ...query.rows.map(async row => {
                const encoded = await this.encodeItem(row, row.$id);
                entityStorage.set(row.$id, encoded);
            })
        ]);
    }

    private async decodeQueryResult<T extends Identifiable>(
        encodedQuery: Uint8Array,
        entityStorage: PersistedService<Uint8Array>
    ): Promise<StoredQueryResult<T> | undefined> {
        const query = await this.decodeItem<StoredQueryResult<string>>(encodedQuery, 'query');
        
        const rows = await Promise.all(
            query.rows.map(async id => {
                const data = entityStorage.get(id);
                if (!data) return null;
                try {
                    return await this.decodeItem<T>(data, id);
                } catch {
                    return null;
                }
            })
        );

        return {
            rows: rows.filter(Boolean) as T[],
            total: query.total,
            hasNextPage: query.hasNextPage,
            hasPreviousPage: query.hasPreviousPage
        };
    }

    public async getQueryResult<T extends Identifiable>(
        queryHash: string,
        entity: string
    ): Promise<SelectResult<T> | undefined> {
        const entityStorage = this.getEntityStorage(entity);
        await this.ensureStoragesLoaded(this.queryStorage, entityStorage);
        
        const encodedQuery = this.queryStorage.get(queryHash);
        if (!encodedQuery) return undefined;
        
        return this.withRetry(async () => this.decodeQueryResult<T>(encodedQuery, entityStorage));
    }

    /**
     * Delete a query from the storage
     * @param queryHash - The hash of the query
     * @param entity - The entity for the query
     */
    public async deleteQuery(queryHash: string, entity: string): Promise<void> {
        const entityStorage = this.getEntityStorage(entity);
        await Promise.all([
            this.queryStorage.waitForLoaded(),
            entityStorage.waitForLoaded()
        ]);

        const encodedQuery = this.queryStorage.get(queryHash);
        if (!encodedQuery) return;
        
        try {
            const query = await this.decodeItem<SelectResult<string>>(encodedQuery, 'query');
            
            // Get all other queries' rows in a single pass without unnecessary async operations
            const referencedRows = new Set(
                (await Promise.all(
                    Object.entries(this.queryStorage.value)
                        .filter(([hash]) => hash !== queryHash)
                        .map(async ([_, value]) => {
                            try {
                                return (await this.decodeItem<SelectResult<string>>(value, 'query')).rows;
                            } catch {
                                return [];
                            }
                        })
                )).flat()
            );

            // Delete rows that aren't referenced by other queries
            for (const row of query.rows) {
                if (!referencedRows.has(row)) {
                    entityStorage.delete(row);
                }
            }

            this.queryStorage.delete(queryHash);
        } catch (error) {
            throw new Error(`Failed to decode query for deletion ${queryHash}: ${error['message']}`);
        }
    }

    /**
     * Add a method to clean up unused storages
     * @param maxAgeMs - The maximum age in milliseconds
     */
    public async cleanupUnusedStorages(maxAgeMs: number = 30 * 60 * 1000): Promise<void> {
        return this.withRetry(async () => {
            const now = Date.now();
            const entriesToRemove: string[] = [];

            for (const [entity, { lastAccessed }] of this.entityDataStorages) {
                if (now - lastAccessed > maxAgeMs) {
                    entriesToRemove.push(entity);
                }
            }

            entriesToRemove.forEach(entity => this.entityDataStorages.delete(entity));
        });
    }

    private async withRetry<T>(
        operation: () => Promise<T>,
        options: StorageRetryOptions = {}
    ): Promise<T> {
        const { maxRetries = 3, delayMs = 1000 } = options;
        let lastError: Error = new Error('Unknown error');
        
        for (let attempt = 0; attempt < maxRetries; attempt++) {
            try {
                return await operation();
            } catch (error) {
                lastError = error instanceof Error ? error : new Error(String(error));
                if (attempt < maxRetries - 1) {
                    await new Promise(resolve => setTimeout(resolve, delayMs * Math.pow(2, attempt)));
                }
            }
        }
        
        throw lastError;
    }

    private async safeOperation<T>(
        operation: () => Promise<T>,
        errorContext: string
    ): Promise<T> {
        return this.withRetry(async () => {
            try {
                return await operation();
            } catch (error) {
                throw new Error(`${errorContext}: ${error instanceof Error ? error.message : 'Unknown error'}`);
            }
        });
    }

    private async decodeItem<T>(data: Uint8Array, id: string): Promise<T> {
        return this.safeOperation(
            async () => JSON.parse(textEncoder.decode(data)) as T,
            `Failed to decode item ${id}`
        );
    }

    private async encodeItem(item: any, id: string): Promise<Uint8Array> {
        return this.safeOperation(
            async () => textEncoder.encode(JSON.stringify(item)),
            `Failed to encode item ${id}`
        );
    }

    private initializeStorage(storeName: string, options: Partial<StorageOptions> = {}): PersistedService<Uint8Array> {
        return new PersistedService<Uint8Array>({
            params: {
                dbName: this.dbName,
                storeName,
                encryptionKey: this.storagePassphrase
            },
            storage: this.storage,
            ...options
        });
    }

    private async ensureStoragesLoaded(...storages: PersistedService<Uint8Array>[]): Promise<void> {
        await this.withRetry(
            async () => {
                const loadPromises = storages.map(storage => storage.waitForLoaded());
                await Promise.all(loadPromises);
            },
            { maxRetries: 5, delayMs: 500 }
        );
    }
}
