import { SocketClientOptions } from '@topgunbuild/socket';
import { DataNode } from '@topgunbuild/store';

export interface ClientOptions
{
    peers?: PeerOption[];
    dbDirectory?: string;
    rowLimit?: number;
}

export type SelectCb = (nodes: DataNode[]) => void;
export type MessageCb = (msg: any) => void;
export type PeerOption = string|SocketClientOptions;
