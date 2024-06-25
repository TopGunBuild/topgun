import { SocketClientOptions } from '@topgunbuild/socket';

export interface ClientOptions
{
    peers?: PeerOption[];
    dbDirectory?: string;
    rowLimit?: number;
}

export type QueryCb<T> = (value: T) => void;
export type MessageCb = (msg: any) => void;
export type PeerOption = string|SocketClientOptions;

