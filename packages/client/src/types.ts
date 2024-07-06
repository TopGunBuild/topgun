import { SocketClientOptions } from '@topgunbuild/socket';
import { DataNode, DataValue } from '@topgunbuild/store';

export interface ClientOptions
{
    peers?: PeerOptions[];
    dbDirectory?: string;
    rowLimit?: number;
}

export interface ConnectorSendOptions
{
    cb?: MessageCb;
    once?: boolean;
}

export type QueryCb<T> = (value: T) => void;
export type MessageCb = (msg: any) => void;
export type PeerOptions = string|SocketClientOptions;
export type DataType = DataNode[]|DataNode|DataValue;

export enum ClientEvents
{
    storeInit = 'storeInit'
}

