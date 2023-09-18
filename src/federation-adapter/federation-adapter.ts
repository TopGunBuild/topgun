import { TGFederatedAdapterOptions } from './types';
import { TGGraphAdapter, TGGraphData, TGOptionsGet } from '../types';
import { createSoul } from '../utils';
import { Writer } from './write/writer';
import { PEER_SYNC_SOUL } from './constants';
import { TGPeers } from './peers';
import { ConnectToPeer } from './sync/connect-to-peer';
import { SyncWithPeer } from './sync/sync-with-peer';

export class TGFederationAdapter implements TGGraphAdapter
{
    internal: TGGraphAdapter;
    peers: TGPeers;
    persistence: TGGraphAdapter;
    options: TGFederatedAdapterOptions;

    private readonly writer: Writer;

    /**
     * Constructor
     */
    constructor(
        internal: TGGraphAdapter,
        peers: TGPeers,
        persistence: TGGraphAdapter,
        options: TGFederatedAdapterOptions
    )
    {
        const defaultOptions: TGFederatedAdapterOptions = {
            backSync         : 1000 * 60 * 60 * 24, // 24 hours
            batchInterval    : 500,
            maxStaleness     : 1000 * 60 * 60 * 24,
            maintainChangelog: true,
            putToPeers       : false
        };

        this.internal    = internal;
        this.peers       = peers;
        this.persistence = persistence;
        this.options     = Object.assign(defaultOptions, options || {});
        this.writer      = new Writer(this.internal, this.persistence, this.peers, this.options);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async get(getOpts: TGOptionsGet)
    {
        console.log('get');
        await this.writer.updateFromPeers(getOpts);
        return this.internal.get(getOpts);
    }

    async put(data: TGGraphData)
    {
        console.log('put');
        const diff = await this.persistence.put(data);

        if (!diff)
        {
            return diff
        }

        if (this.options.maintainChangelog)
        {
            this.writer.updateChangelog(diff);
        }

        if (this.options.putToPeers)
        {
            this.writer.updatePeers(diff, this.peers);
        }

        return diff
    }

    async syncWithPeer(peerName: string, from: string): Promise<string>
    {
        console.log('syncWithPeer ', peerName);
        return SyncWithPeer.sync(peerName, from, this.peers, this.persistence, this.options, this.writer);
    }

    connectToPeers(): () => void
    {
        if (this.peers.size)
        {
            console.log('connectToPeers');
        }

        const connectors: ConnectToPeer[] = [];

        this.peers.getPeerNames().forEach(async (peerName) =>
        {
            const key       = await this.#getPeerSyncDate(peerName);
            const connector = new ConnectToPeer(peerName, key, this.peers, this.persistence, this.options, this.writer);
            connector.connect();
            connectors.push(connector);
        });

        return () => connectors.forEach(c => c.disconnect());
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #getPeerSyncDate(peerName: string): Promise<string>
    {
        console.log('getPeerSyncDate ', peerName);
        const peerSyncSoul = createSoul(PEER_SYNC_SOUL, peerName);
        const graph        = await this.internal.get({
            '#': peerSyncSoul
        });

        if (graph && graph[peerSyncSoul])
        {
            console.log(graph);
        }

        return new Date(Date.now() - this.options.backSync).getTime().toString(); // yesterday
    }
}