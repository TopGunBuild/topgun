import { SocketClientOptions } from '@topgunbuild/socket';
import { DataNode } from '@topgunbuild/store';
import { Query, SelectOptions, Sort } from '@topgunbuild/transport';

export interface ClientOptions
{
    peers?: PeerOption[];
    dbDirectory?: string;
    rowLimit?: number;
}

export type SelectCb = (nodes: DataNode[]) => void;
export type MessageCb = (msg: any) => void;
export type PeerOption = string|SocketClientOptions;
export type SqlSelectOptions = SelectOptions&{
    fields?: string[];
    query?: Query[];
    sort?: Sort[];
    limit?: number,
    offset?: number
};
