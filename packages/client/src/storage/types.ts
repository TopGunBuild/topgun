import { Password } from "@topgunbuild/crypto";

/**
 * The valid key for IDB
 */
export type IDBValidKey = number | string | Uint8Array;

/**
 * The storage params
 */
export type StorageParams<T, U> = { 
    dbName?: string, 
    storeName?: string, 
    encryptionKey?: Password,
    writeMiddleware?: (value: T) => U,
    readMiddleware?: (value: U) => T,
};

/**
 * The storage adapter
 */
export interface StorageAdapter<T> {
    get(key: IDBValidKey): Promise<T>;
    put(key: IDBValidKey, value: T): Promise<void>;
    getAll(): Promise<Record<string, T>>;
    delete(key: IDBValidKey): Promise<void>;
    close(): Promise<void>;
}

/**
 * The storage derived
 */
export type StorageDerived<T, R> = new (params: StorageParams<T, R>) => StorageAdapter<T>;

/**
 * The merge function
 */
export type MergeFunction<T> = (fromStorage: T, currentValue: T) => T;
