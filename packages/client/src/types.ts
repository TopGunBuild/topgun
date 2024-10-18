import { Action } from "@topgunbuild/types";

export type ClientConfig = {
    websocketURIs?: string[];
    appId?: string;
    windowNetworkListener?: NetworkListenerDerived;
    storage?: StorageDerived;
};

export interface NetworkListenerAdapter {
    isOnline(): boolean;
    listen(f: (isOnline: boolean) => void): () => void;
}
export type NetworkListenerDerived = new () => NetworkListenerAdapter;

export type IDBValidKey = number | string | Uint8Array;
export type StorageParams = { dbName?: string, storeName?: string };

export interface StorageAdapter<T> {
    get(key: IDBValidKey): Promise<T>;
    put(key: IDBValidKey, value: T): Promise<void>;
    getAll(): Promise<T[]>;
    update(key: IDBValidKey, updater: (val: Partial<T>) => T): Promise<void>;
    delete(key: IDBValidKey): Promise<void>;
}
export type StorageDerived = new (params: StorageParams) => StorageAdapter<any>; // { new (): StorageAdapter<any> } & typeof StorageAdapter;

export type QueryCb<T> = (value: T) => void;
export interface QueryState<T> {
    action: Action;
    cbs: QueryCb<T>[];
    result: T;
    resultHash: string;
}

export type NetworkStatus = 'online' | 'offline';
