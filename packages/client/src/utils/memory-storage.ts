import { StorageAdapter } from "../types";

/**
 * MemoryStorage is a storage adapter that uses an in-memory Map to store data.
 * @template T The type of the data to store
 */
export class MemoryStorage<T> extends StorageAdapter<T>
{
    private storage: Map<string, T>;

    /**
     * Create a new MemoryStorage
     * @param params 
     */
    constructor(params: { dbName?: string, storeName?: string } = {}) {
        super(params);
        this.storage = new Map<string, T>();
    }

    /**
     * Get a value from the storage
     * @param key 
     * @returns 
     */
    async get(key: string): Promise<T | undefined> {
        return this.storage.get(key);
    }

    /**
     * Put a value into the storage
     * @param key 
     * @param value 
     */
    async put(key: string, value: T): Promise<void> {
        this.storage.set(key, value);
    }

    /**
     * Delete a value from the storage
     * @param key 
     */
    async delete(key: string): Promise<void> {
        this.storage.delete(key);
    }   

    /**
     * Get all values from the storage
     * @returns 
     */
    async getAll(): Promise<T[]> {
        return Array.from(this.storage.values());
    }

    /**
     * Update a value in the storage
     * @param key 
     * @param updater 
     */
    async update(key: string, updater: (val: Partial<T>) => T): Promise<void> {
        const value = await this.get(key);
        if (value) {
            const updatedValue = updater(value);
            this.storage.set(key, updatedValue);
        }
    }
}
