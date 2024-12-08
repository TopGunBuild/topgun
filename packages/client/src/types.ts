import { StorageDerived } from "./storage/types";
import { FilterGroup } from "@topgunbuild/collections";
import { MessageRow, Password, SelectAction, SelectOptions, SelectResult, StoreItem } from "@topgunbuild/models";

/**
 * The configuration for the client
 */
export type ClientConfig = {
    websocketURI?: string;
    appId?: string;
    windowNetworkListener?: NetworkListenerDerived;
    storage?: StorageDerived<any, any>;
    storagePassphrase?: Password;
    remoteQueryTimeout?: number;
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
    filterOptions: FilterGroup;
    cbs: QueryCb<T>[];
    result: SelectResult<T> | null;
}

/**
 * The network status
 */
export type NetworkStatus = 'online' | 'offline';

export interface IChannelAPI {
    subscribeMessages(options: SelectOptions, cb: QueryCb<SelectResult<MessageRow>>): () => void;
    addMessages<T extends StoreItem>(messages: T[]): Promise<void>;
}

/**
 * The parameters for creating a team
 */
export interface CreateTeamParams {
    name: string;
    description?: string;
    seed?: string;
}

// export interface GetByIdResult<T> {
//     data: T | null;
//     source: 'local' | 'remote';
// }