import { SocketClientOptions } from '@topgunbuild/socket';
import { DataNode, DataValue } from '@topgunbuild/store';

export interface ClientOptions
{
    peers?: PeerOption[];
    dbDirectory?: string;
    rowLimit?: number;
}

export type QueryCb<T> = (value: T) => void;
export type MessageCb = (msg: any) => void;
export type PeerOption = string|SocketClientOptions;
export type DataType = DataNode[]|DataNode|DataValue;

export enum ClientEvents
{
    storeInit = 'storeInit'
}

export enum QueryHandlerEvents
{
    storeInit        = 'storeInit',
    localDataFetched = 'localDataFetched'
}
