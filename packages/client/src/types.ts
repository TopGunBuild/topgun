import { SelectQuery } from "@topgunbuild/types";
import { StorageDerived } from "./storage/types";

/**
 * The configuration for the client
 */
export type ClientConfig = {
    websocketURI?: string;
    appId?: string;
    windowNetworkListener?: NetworkListenerDerived;
    storage?: StorageDerived<any>;
};

/**
 * The network listener adapter
 */
export interface NetworkListenerAdapter {
    isOnline(): boolean;
    listen(f: (isOnline: boolean) => void): () => void;
}
export type NetworkListenerDerived = new () => NetworkListenerAdapter;

/**
 * The callback for a query
 */
export type QueryCb<T> = (value: T) => void;

/**
 * The state of a query
 */
export interface QueryState<T> {
    query: SelectQuery;
    cbs: QueryCb<T>[];
    result: T;
    resultHash: string;
}

/**
 * The network status
 */
export type NetworkStatus = 'online' | 'offline';
