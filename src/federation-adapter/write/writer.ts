import { TGGraphAdapter, TGGraphData, TGOptionsGet } from '../../types';
import { createSoul, uuidv4 } from '../../utils';
import { CHANGELOG_SOUL, PEER_SYNC_SOUL } from '../constants';
import { TGPeers } from '../peers';
import { TGFederatedAdapterOptions } from '../types';

export class Writer
{
    /**
     * Constructor
     */
    constructor(
        private readonly internalAdapter: TGGraphAdapter,
        private readonly persistence: TGGraphAdapter,
        private readonly peers: TGPeers,
        private readonly options: TGFederatedAdapterOptions
    )
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async updatePeers(data: TGGraphData, peers: TGPeers): Promise<void>
    {
        if (peers.size)
        {
            console.log('updatePeers', peers.size, data);
            await Promise.all(
                peers.getEntries().map(([name, peer]) =>
                    peer
                        .put(data)
                        .catch((err) =>
                        {
                            console.warn('Failed to update peer', name, err.stack || err, data)
                        })
                )
            );
        }
    }

    async updateChangelog(diff: TGGraphData): Promise<void>
    {
        console.log('updateChangelog', diff);
        const now  = new Date().getTime();
        const soul = createSoul(CHANGELOG_SOUL, now, uuidv4());
        await this.internalPut({
            [soul]: {
                _: {
                    '#': soul,
                    '>': {
                        diff: now
                    }
                },
                diff
            }
        });
    }

    async updateFromPeers(getOpts: TGOptionsGet): Promise<void>
    {
        if (this.peers.size > 0)
        {
            console.log('updateFromPeers', getOpts);
            await Promise.all(
                this.peers.getPeerNames().map(peerName =>
                    this.#updateFromPeer(peerName, getOpts)
                )
            );
        }
    }

    async internalPut(data: TGGraphData): Promise<TGGraphData|null>
    {
        return await this.internalAdapter.put(data);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #updateFromPeer(peerName: string, getOpts: TGOptionsGet): Promise<void>
    {
        console.log('updateFromPeer', getOpts);
        const soul = getOpts['#'];

        if (soul.startsWith(CHANGELOG_SOUL) || soul.startsWith(PEER_SYNC_SOUL))
        {
            return
        }

        const peerSoul  = createSoul('peers', peerName);
        const status    = await this.internalAdapter.get({
            '#': createSoul(peerSoul, soul)
        });
        const now       = new Date().getTime();
        const staleness = now - ((status && status._['>'][soul]) || 0);

        const { maxStaleness, maintainChangelog, putToPeers } = this.options;

        if (staleness < maxStaleness)
        {
            return
        }

        const peer  = this.peers.get(peerName);
        const graph = await peer.get({
            '#': soul
        });

        if (graph)
        {
            try
            {
                const diff = await this.persistence.put(graph);

                if (diff)
                {
                    if (maintainChangelog)
                    {
                        this.updateChangelog(diff);
                    }

                    if (putToPeers)
                    {
                        const otherPeers = this.peers.getOtherPeers(peerName);
                        this.updatePeers(diff, otherPeers);
                    }
                }
            }
            catch (e: any)
            {
                console.warn('Error updating from peer', {
                    error: e.stack,
                    peerName,
                    soul
                });
            }
        }

        await this.internalPut({
            [peerSoul]: {
                _: {
                    '#': peerSoul,
                    '>': {
                        [soul]: now
                    }
                },
                [soul]: Object.keys(graph).length > 0
            }
        });
    }
}
