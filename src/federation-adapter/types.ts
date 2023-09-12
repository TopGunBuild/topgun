import { TGGraphData } from '../types';
import { WebSocketAdapter } from '../web-socket-adapter';

export type TGPeerMap = Map<string, WebSocketAdapter>;

export type TGChangeSetEntry = readonly [string, TGGraphData];

export interface TGFederatedAdapterOptions
{
    readonly backSync?: number;
    readonly maxStaleness?: number;
    readonly maintainChangelog?: boolean;
    readonly putToPeers?: boolean;
    readonly batchInterval?: number;
}

