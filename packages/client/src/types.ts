import { StorageDerived } from "./storage/types";
import { FilteringCriteriaTree } from "@topgunbuild/collections";
import { Password, SelectAction, SelectResult } from "@topgunbuild/models";

/**
 * The configuration for the client
 */
export type ClientConfig = {
    websocketURI?: string;
    appId?: string;
    windowNetworkListener?: NetworkListenerDerived;
    storage?: StorageDerived<any>;
    storagePassphrase?: Password;
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
export interface QueryState<T extends { id: string }> {
    query: SelectAction;
    filterCriteria: FilteringCriteriaTree;
    cbs: QueryCb<T>[];
    result: SelectResult<T> | null;
}

/**
 * The network status
 */
export type NetworkStatus = 'online' | 'offline';
