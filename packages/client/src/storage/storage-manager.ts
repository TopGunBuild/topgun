import { Identifiable, IPayload, ISelectResult } from "@topgunbuild/types";
import { PersistedService } from "./persisted-service";
import { StorageDerived } from "./types";
import { EncryptedPayload } from "@topgunbuild/models";

/**
 * The storage manager
 */
export class StorageManager {
    private readonly entityDataStorages: Map<string, PersistedService<any>> = new Map();
    private readonly queryStorage: PersistedService<ISelectResult<string>>;
    private readonly pendingActionsStorage: PersistedService<any>;
    private readonly storage: StorageDerived<any>;
    private readonly dbName: string;

    /**
     * Constructor
     * @param dbName - The name of the database
     * @param storage - The storage adapter
     */
    constructor(
        dbName: string,
        storage: StorageDerived<any>
    ) {
        this.dbName = dbName;
        this.storage = storage;

        // Initialize common storages
        this.queryStorage = new PersistedService({
            params: { dbName, storeName: 'queries' },
            storage,
        });

        this.pendingActionsStorage = new PersistedService({
            params: { dbName, storeName: 'pendingActions' },
            storage,
        });
    }

    /**
     * Get the entity storage
     * @param entity - The entity
     * @returns The entity storage
     */
    private getEntityStorage(entity: string): PersistedService<any> {
        if (!this.entityDataStorages.has(entity)) {
            this.entityDataStorages.set(
                entity,
                new PersistedService({
                    params: { dbName: this.dbName, storeName: `data_${entity}` },
                    storage: this.storage,
                    merge: (fromStorage, currentValue) => ({
                        ...(fromStorage || {}),
                        ...(currentValue || {})
                    })
                })
            );
        }
        return this.entityDataStorages.get(entity)!;
    }

    /**
     * Get all pending actions from storage
     * @returns An array of pending actions
     */
    public async getAllPendingActions(): Promise<any[]> {
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
    public async getPendingAction(id: string): Promise<any | undefined> {
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
     * Update or insert data into entity storage
     * @param entity - The entity type
     * @param data - The data to store
     */
    public async upsert<T extends Identifiable>(entity: string, data: T[]): Promise<void> {
        const entityStorage = this.getEntityStorage(entity);
        await entityStorage.waitForLoaded();
        
        for (const item of data) {
            entityStorage.set(item.$id, item);
        }
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
     * Get an item from entity storage
     * @param entity - The entity type
     * @param id - The id of the item
     * @returns The item
     */
    public async get<T extends Identifiable>(entity: string, id: string): Promise<T | undefined> {
        const entityStorage = this.getEntityStorage(entity);
        await entityStorage.waitForLoaded();
        return entityStorage.get(id) as T | undefined;
    }

    /**
     * Put a query into the storage
     * @param queryHash - The hash of the query
     * @param query - The query result
     * @param entity - The entity for the query
     */
    public saveQueryResult<T extends { id: string }>(queryHash: string, query: ISelectResult<T>, entity: string) {
        const queryResult: ISelectResult<string> = {
            rows: query.rows.map(row => row.id),
            total: query.total,
            hasNextPage: query.hasNextPage,
            hasPreviousPage: query.hasPreviousPage,
        };
        this.queryStorage.set(queryHash, queryResult);
        const entityStorage = this.getEntityStorage(entity);
        query.rows.forEach(row => entityStorage.set(row.id, row));
    }

    /**
     * Get a query from the storage
     * @param queryHash - The hash of the query
     * @param entity - The entity for the query
     */
    public async getQueryResult<T extends Identifiable>(
        queryHash: string,
        entity: string
    ): Promise<ISelectResult<T> | undefined> {
        const entityStorage = this.getEntityStorage(entity);
        await Promise.all([
            this.queryStorage.waitForLoaded(),
            entityStorage.waitForLoaded()
        ]);
        
        const query = this.queryStorage.get(queryHash);
        if (!query) return undefined;
        return {
            rows: query.rows.map(id => entityStorage.get(id) as T).filter(Boolean) as T[],
            total: query.total,
            hasNextPage: query.hasNextPage,
            hasPreviousPage: query.hasPreviousPage
        };
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

        const query = this.queryStorage.get(queryHash);
        if (query) {
            const otherQueries = Object.entries(this.queryStorage.value)
                .filter(([hash]) => hash !== queryHash)
                .map(([_, value]) => value);

            const referencedRows = new Set(otherQueries.flatMap(q => q.rows));

            query.rows.forEach(row => {
                if (!referencedRows.has(row)) {
                    entityStorage.delete(row);
                }
            });

            this.queryStorage.delete(queryHash);
        }
    }
}
