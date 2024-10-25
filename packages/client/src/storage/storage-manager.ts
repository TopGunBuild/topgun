import { IEncryptedAction, QueryResult, SelectQuery } from "@topgunbuild/types";
import { PersistedService } from "./persisted-service";
import { StorageDerived } from "./types";

/**
 * The storage manager
 */
export class StorageManager {
    private dataStorage: PersistedService<any>;
    private queryStorage: PersistedService<QueryResult<string>>;
    private pendingActionsStorage: PersistedService<IEncryptedAction>;

    /**
     * Constructor
     * @param dbName - The name of the database
     * @param storage - The storage adapter
     */
    constructor(
        dbName: string,
        storage: StorageDerived<any>
    ) {
        // Data storage
        this.dataStorage = new PersistedService({
            params: { dbName, storeName: 'data' },
            storage,
            merge: (fromStorage, currentValue) => {
                // Return empty object if both are null/undefined
                if (!fromStorage && !currentValue) {
                    return {};
                }
                // Return currentValue if fromStorage is null/undefined
                if (!fromStorage) {
                    return currentValue;
                }
                // Return fromStorage if currentValue is null/undefined
                if (!currentValue) {
                    return fromStorage;
                }
                // Merge the data when both exist
                return { ...fromStorage, ...currentValue };
            }
        }); 

        // Queries storage
        this.queryStorage = new PersistedService({
            params: { dbName, storeName: 'queries' },
            storage,
        });

        // Pending actions storage
        this.pendingActionsStorage = new PersistedService({
            params: { dbName, storeName: 'pendingActions' },
            storage,
        });
    }
    /**
     * Get all pending actions from storage
     * @returns An array of pending actions
     */
    public async getAllPendingActions(): Promise<IEncryptedAction[]> {
        await this.pendingActionsStorage.waitForLoaded();
        return Object.values(this.pendingActionsStorage.value);
    }

    /**
     * Put a pending action into the storage
     * @param action - The action
     */
    public putPendingAction(action: IEncryptedAction) {
        this.pendingActionsStorage.set(action.hash, action);
    }

    /**
     * Get a pending action from the storage
     * @param hash - The hash of the action
     */
    public async getPendingAction(hash: string): Promise<IEncryptedAction | undefined> {
        await this.pendingActionsStorage.waitForLoaded();
        return this.pendingActionsStorage.get(hash);
    }

    /**
     * Delete a pending action from the storage
     * @param hash - The hash of the action
     */
    public deletePendingAction(hash: string) {
        this.pendingActionsStorage.delete(hash);
    }

    /**
     * Put a query into the storage
     * @param queryHash - The hash of the query
     * @param query - The query result
     */
    public putQuery<T extends { id: string }>(queryHash: string, query: QueryResult<T>) {
        const queryResult: QueryResult<string> = {
            rows: query.rows.map(row => row.id),
            total: query.total,
            hasNextPage: query.hasNextPage,
            hasPreviousPage: query.hasPreviousPage,
        };
        this.queryStorage.set(queryHash, queryResult);
        query.rows.forEach(row => this.dataStorage.set(row.id, row));
    }

    /**
     * Get a query from the storage
     * @param queryHash - The hash of the query
     */
    public async getQuery<T extends { id: string }>(queryHash: string): Promise<QueryResult<T> | undefined> {
        await Promise.all([
            this.queryStorage.waitForLoaded(),
            this.dataStorage.waitForLoaded()
        ]);
        
        const query = this.queryStorage.get(queryHash);
        if (!query) return undefined;
        return {
            rows: query.rows.map(id => this.dataStorage.get(id) as T),
            total: query.total,
            hasNextPage: query.hasNextPage,
            hasPreviousPage: query.hasPreviousPage
        };
    }

    /**
     * Delete a query from the storage
     * @param queryHash - The hash of the query
     */
    public async deleteQuery(queryHash: string): Promise<void> {
        await Promise.all([
            this.queryStorage.waitForLoaded(),
            this.dataStorage.waitForLoaded()
        ]);

        const query = this.queryStorage.get(queryHash);
        if (query) {
            // Get all queries except the one being deleted
            const otherQueries = Object.entries(this.queryStorage.value)
                .filter(([hash]) => hash !== queryHash)
                .map(([_, value]) => value);

            // Create a set of all row IDs referenced in other queries
            const referencedRows = new Set(otherQueries.flatMap(q => q.rows));

            // Only delete rows that aren't referenced elsewhere
            query.rows.forEach(row => {
                if (!referencedRows.has(row)) {
                    this.dataStorage.delete(row);
                }
            });

            this.queryStorage.delete(queryHash);
        }
    }
}
