import { isNotEmptyObject } from '@topgunbuild/typed';
import { TGChangeSetEntry, TGFederatedAdapterOptions } from './types';
import { TGGraphAdapter, TGGraphData, TGOptionsGet } from '../types';
import { createSoul } from '../utils';
import { createLex } from '../client/link/lex';
import { Writer } from './writer';
import { CHANGELOG_SOUL, PEER_SYNC_SOUL } from './constants';
import { BatchWriter } from './batch-writer';
import { TGPeers } from './peers';

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
        adapterOpts: TGFederatedAdapterOptions
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
        this.options     = Object.assign(defaultOptions, adapterOpts || {});
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
        const peer       = this.peers.get(peerName);
        const otherPeers = this.peers.getOtherPeers(peerName);
        const getNext    = this.#getChangesetFeed(peer, from);
        let entry: TGChangeSetEntry|null;

        const batch = new BatchWriter(otherPeers, this.persistence, this.options, this.writer);

        let lastSeenKey: string = from;

        while ((entry = await getNext()))
        {
            const [key, changes] = entry;

            if (key > lastSeenKey)
            {
                batch.queueDiff(changes);
                lastSeenKey = key
            }
        }

        if (lastSeenKey > from)
        {
            try
            {
                console.log('writing batch', peerName, lastSeenKey);
                await batch.writeBatch();
                console.log('wrote batch', peerName, lastSeenKey);
            }
            catch (e: any)
            {
                // tslint:disable-next-line: no-console
                console.error('Error syncing with peer', peerName, e.stack);
            }

            const peerSyncSoul = createSoul(PEER_SYNC_SOUL, peerName);
            await this.writer.internalPut({
                [peerSyncSoul]: {
                    _: {
                        '#': peerSyncSoul,
                        '>': {
                            lastSeenKey: new Date().getTime()
                        }
                    },
                    lastSeenKey: lastSeenKey
                }
            })
        }

        return lastSeenKey
    }

    getChangesetFeed(from: string): () => Promise<TGChangeSetEntry|null>
    {
        return this.#getChangesetFeed(this.internal, from);
    }

    connectToPeers(): () => void
    {
        console.log('connectToPeers');

        const disconnectors: Array<() => void> = [];

        this.peers.getPeerNames().forEach(async (peerName) =>
        {
            const key = await this.#getPeerSyncDate(peerName);

            disconnectors.push(
                this.#connectToPeer(peerName, key)
            )
        });

        return () => disconnectors.map(dc => dc());
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

    #connectToPeer(peerName: string, from: string): () => void
    {
        console.log('connectToPeer ', peerName);
        const peer = this.peers.get(peerName);

        if (!peer || !peer?.onChange)
        {
            throw new Error(`Unconnectable peer ${peerName}`);
        }

        const otherPeers = this.peers.getOtherPeers(peerName);
        const batch      = new BatchWriter(otherPeers, this.persistence, this.options, this.writer);

        let disconnector: () => void;
        let batchTimeout: any;

        (async () =>
        {
            // Catch up in batches before establishing connection
            let lastKey   = await this.syncWithPeer(peerName, from);
            let syncedKey = lastKey;

            const writeBatch = async (): Promise<void> =>
            {
                if (syncedKey === lastKey)
                {
                    if (this.options.batchInterval)
                    {
                        batchTimeout = setTimeout(writeBatch, this.options.batchInterval)
                    }
                    return
                }

                syncedKey = lastKey;

                await batch.writeBatch();
                const peerSyncSoul = createSoul(PEER_SYNC_SOUL, peerName);
                await this.writer.internalPut({
                    [peerSyncSoul]: {
                        _: {
                            '#': peerSyncSoul,
                            '>': {
                                lastKey: new Date().getTime()
                            }
                        },
                        lastKey: lastKey
                    }
                });

                if (this.options.batchInterval)
                {
                    batchTimeout = setTimeout(writeBatch, this.options.batchInterval)
                }
            };

            disconnector = peer.onChange(([key, changes]) =>
            {
                try
                {
                    batch.queueDiff(changes);
                    lastKey = key;
                    if (!this.options.batchInterval)
                    {
                        writeBatch()
                    }
                }
                catch (e: any)
                {
                    console.warn('Error syncing from peer', peerName, e.stack)
                }
            }, lastKey);

            if (this.options.batchInterval)
            {
                writeBatch()
            }
        })();

        return () =>
        {
            disconnector && disconnector();
            batchTimeout && clearTimeout(batchTimeout);
        }
    }

    #getChangesetFeed(peer: TGGraphAdapter, from: string): () => Promise<TGChangeSetEntry|null>
    {
        console.log('getChangesetFeed');
        let lastKey                                      = from;
        const changes: TGChangeSetEntry[]                = [];
        let graphPromise: Promise<TGGraphData|null>|null = null;

        return async (): Promise<readonly [string, TGGraphData]|null> =>
        {
            if (!changes.length && !graphPromise)
            {
                graphPromise = peer.get(
                    createLex(CHANGELOG_SOUL).start(lastKey).getQuery()
                );
                const graph  = await graphPromise;
                // console.log(graph, createLex(CHANGELOG_SOUL).start(lastKey).getQuery());
                graphPromise = null;

                if (isNotEmptyObject(graph))
                {
                    console.log(graph);
                    for (const key in graph)
                    {
                        if (key && key !== '_')
                        {
                            changes.splice(0, 0, [key, graph[key]]);
                            lastKey = key
                        }
                    }
                }
            }
            else if (graphPromise)
            {
                await graphPromise;
                graphPromise = null
            }

            const entry = changes.pop();
            return entry || null
        }
    }
}