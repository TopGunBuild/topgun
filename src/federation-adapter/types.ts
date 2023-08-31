import { TGGraphAdapter } from '../types';

export type TGPeerSet = Record<string, TGGraphAdapter>;

export interface TGFederatedAdapterOptions
{
    readonly backSync?: number;
    readonly maxStaleness?: number;
    readonly maintainChangelog?: boolean;
    readonly putToPeers?: boolean;
    readonly batchInterval?: number;
}
