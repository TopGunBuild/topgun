import { TGGraphAdapter, TGGraphData } from '../types';
import { TGWebSocketGraphConnector } from '../client/transports/web-socket-graph-connector';

export type TGPeerSet = Record<string, TGWebSocketGraphConnector>;

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
