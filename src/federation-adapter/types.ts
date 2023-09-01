import { TGGraphAdapter, TGGraphData } from '../types';

export type TGPeerSet = Record<string, TGGraphAdapter>;

export type TGChangeSetEntry = readonly [string, TGGraphData];

export interface TGFederatedAdapterOptions
{
    readonly backSync?: number;
    readonly maxStaleness?: number;
    readonly maintainChangelog?: boolean;
    readonly putToPeers?: boolean;
    readonly batchInterval?: number;
}

export interface TGFederatedGraphAdapter extends TGGraphAdapter
{
    readonly syncWithPeers: () => Promise<void>
    readonly connectToPeers: () => () => void
    readonly getChangesetFeed: (
        from: string
    ) => () => Promise<TGChangeSetEntry|null>
}
