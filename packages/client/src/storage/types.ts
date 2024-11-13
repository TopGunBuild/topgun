import { Password } from "@topgunbuild/crypto";

/**
 * The valid key for IDB
 */
export type IDBValidKey = number | string | Uint8Array;

/**
 * The storage params
 */
export type StorageParams = { dbName?: string, storeName?: string, encryptionKey?: Password };

/**
 * The storage adapter
 */
export interface StorageAdapter<T> {
    get(key: IDBValidKey): Promise<T>;
    put(key: IDBValidKey, value: T): Promise<void>;
    getAll(): Promise<Record<string, T>>;
    delete(key: IDBValidKey): Promise<void>;
}

/**
 * The storage derived
 */
export type StorageDerived<T> = new (params: StorageParams) => StorageAdapter<T>;

/**
 * The merge function
 */
export type MergeFunction<T> = (fromStorage: T, currentValue: T) => T;
