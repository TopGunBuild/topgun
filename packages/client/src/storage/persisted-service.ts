import { AsyncQueue } from "@topgunbuild/common";
import { MergeFunction, StorageAdapter, StorageParams, StorageDerived } from "./types";

/**
 * The persisted service
 */
export class PersistedService<T, U> {
    private _persister: StorageAdapter<T>;
    private _loadedCbs: (() => void)[] = [];
    private _queue = new AsyncQueue();
    private _isLoading = true;
    private _merge: MergeFunction<T>;
    public value: Record<string, T>;

    /**
     * Constructor
     * @param params - The parameters
     */
    constructor(params: {
        params: StorageParams<T, U>;
        storage: StorageDerived<T, U>;
        defaultValue?: Record<string, T>;
        merge?: MergeFunction<T>;
    }) {
        this._persister = new params.storage(params.params);
        this.value = params.defaultValue || {};
        this._merge = params.merge || ((fromStorage, currentValue) => currentValue);
        this._load();
    }
    
    /**
     * Gets all values from storage
     * @returns Promise<T[]> Array of all stored values
     */
    public async getAll(): Promise<T[]> {
        await this.waitForLoaded();
        return Object.values(this.value);
    }

    /**
     * Checks if the object is still loading
     */
    public isLoading(): boolean {
        return this._isLoading;
    }

    /**
     * Gets the current value
     */
    public get(key: string): T | undefined {
        return this.value[key];
    }

    /**
     * Sets the value for a key
     */
    public set(key: string, value: T): void {
        if (this._isLoading) {
            this.waitForLoaded().then(() => this.set(key, value));
            return;
        }

        const existingValue = this.value[key];
        if (existingValue) {
            this.value[key] = this._merge(existingValue, value);
        } else {
            this.value[key] = value;
        }

        this._queue.enqueue(() => this._persister.put(key, this.value[key]));
    }

    public delete(key: string): void {
        if (this._isLoading) {
            this.waitForLoaded().then(() => this.delete(key));
            return;
        }
        delete this.value[key];
        this._queue.enqueue(() => this._persister.delete(key));
    }

    /**
     * Waits for the object to finish loading
     */
    public async waitForLoaded(): Promise<void> {
        if (!this._isLoading) {
            return;
        }
        return new Promise<void>((resolve) => {
            this._loadedCbs.push(resolve);
        });
    }

    /**
     * Loads data from storage and merges it with the current value
     */
    private async _load(): Promise<void> {
        this.value = await this._persister.getAll();
        this._isLoading = false;

        for (const cb of this._loadedCbs) {
            cb();
        }
    }
}
