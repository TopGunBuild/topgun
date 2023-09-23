import { isNotEmptyObject } from '@topgunbuild/typed';
import { TGGraphAdapter, TGGraphData, TGOptionsGet } from '../types';
import { TGPeers } from './peers';
import { TGFederatedAdapterOptions } from './types';
import { TGExtendedLoggerType } from '../logger';
import { TGPeer } from './peer';

export class Writer
{
    /**
     * Constructor
     */
    constructor(
        private readonly persistence: TGGraphAdapter,
        private readonly peers: TGPeers,
        private readonly options: TGFederatedAdapterOptions,
        private readonly logger: TGExtendedLoggerType
    )
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async updatePeers(data: TGGraphData, peers: TGPeer[]): Promise<void>
    {
        if (peers.length > 0)
        {
            this.logger.log('updatePeers', data);
            await Promise.all(
                peers.map(peer =>
                    peer
                        .putData(data)
                        .catch((err) =>
                        {
                            this.logger.warn('Failed to update peer', peer.uri, err.stack || err, data)
                        })
                )
            );
        }
    }

    async updateFromPeers(getOpts: TGOptionsGet): Promise<void>
    {
        if (this.peers.size > 0)
        {
            await Promise.all(
                this.peers.getPeers()
                    .filter(peer => peer.isConnected && peer.isOpen)
                    .map(peer => this.#updateFromPeer(peer.uri, getOpts))
            );
        }
    }

    async put(graph: TGGraphData, peerUri: string): Promise<void>
    {
        if (isNotEmptyObject(graph))
        {
            const diff = await this.persistence.put(graph);

            if (diff)
            {
                if (this.options.putToPeers)
                {
                    const otherPeers = this.peers.getOtherPeers(peerUri);
                    this.updatePeers(diff, otherPeers);
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #updateFromPeer(peerUri: string, getOpts: TGOptionsGet): Promise<void>
    {
        this.logger.log('updateFromPeer', getOpts);

        const peer  = this.peers.get(peerUri);
        const graph = await peer.getData(getOpts);

        try
        {
            await this.put(graph, peerUri);
        }
        catch (e)
        {
            this.logger.error('Error updating from peer', {
                error: e.stack,
                peerUri
            });
        }
    }
}
