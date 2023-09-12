import { isNotEmptyObject } from '@topgunbuild/typed';
import { TGChangeSetEntry, TGFederatedAdapterOptions, TGPeerMap } from './types';
import { TGGraphAdapter, TGGraphData, TGOptionsGet } from '../types';
import { createSoul, uuidv4 } from '../utils';
import { diffCRDT, mergeGraph } from '../crdt';
import { createLex } from '../client/link/lex';

const CHANGELOG_SOUL = 'changelog';
const PEER_SYNC_SOUL = 'peersync';

export class TGFederationAdapter implements TGGraphAdapter
{
    internal: TGGraphAdapter;
    peers: TGPeerMap;
    persistence: TGGraphAdapter;
    options: TGFederatedAdapterOptions;

    /**
     * Constructor
     */
    constructor(
        internal: TGGraphAdapter,
        peers: TGPeerMap,
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
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async get(getOpts: TGOptionsGet)
    {
        console.log('get');
        await this.#updateFromPeers(getOpts);
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
            this.#updateChangelog(diff);
        }

        if (this.options.putToPeers)
        {
            this.#updatePeers(diff, this.peers);
        }

        return diff
    }

    async syncWithPeer(peerName: string, from: string): Promise<string>
    {
        console.log('syncWithPeer');
        const peer       = this.peers.get(peerName);
        const otherPeers = this.#getOtherPeers(peerName);
        const getNext    = this.#getChangesetFeed(peer, from);
        let entry: TGChangeSetEntry|null;

        const batch = this.#batchWriter(otherPeers);

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
            await this.internal.put({
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

        this.peers.forEach(async (_, peerName) =>
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
        console.log('getPeerSyncDate');
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
        console.log('connectToPeer');
        const peer = this.peers.get(peerName);

        if (!peer || !peer?.onChange)
        {
            throw new Error(`Unconnectable peer ${peerName}`);
        }

        const otherPeers = this.#getOtherPeers(peerName);
        const batch      = this.#batchWriter(otherPeers);

        let disconnector: () => void;
        let batchTimeout: any;

        (async () =>
        {
            // Catch up in batches before establishing connection
            let lastKey = await this.syncWithPeer(peerName, from);

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
                await this.internal.put({
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

    #batchWriter(peers: TGPeerMap): {
        queueDiff: (changes: TGGraphData) => TGGraphData|undefined
        writeBatch: () => Promise<TGGraphData|null>
    }
    {
        console.log('batchWriter');
        let batch: TGGraphData = {};

        const queueDiff = (changes: TGGraphData): TGGraphData|undefined =>
        {
            const diff = diffCRDT(changes, batch);
            batch      = diff ? mergeGraph(batch, diff, 'mutable') : batch;
            return diff
        };

        const writeBatch = async (): Promise<TGGraphData|null> =>
        {
            if (!Object.keys(batch).length)
            {
                return null
            }
            const toWrite = batch;
            batch         = {};

            const diff = await this.persistence.put(toWrite);

            if (diff)
            {
                if (this.options.maintainChangelog)
                {
                    this.#updateChangelog(diff);
                }

                if (this.options.putToPeers)
                {
                    this.#updatePeers(diff, peers);
                }
            }

            return diff;
        };

        return { queueDiff, writeBatch };
    }

    async #updatePeers(data: TGGraphData, otherPeers: TGPeerMap): Promise<void>
    {
        const entries = Array.from(otherPeers);
        console.log('updatePeers', entries.length, data);

        if (entries.length)
        {
            await Promise.all(
                entries.map(([name, peer]) =>
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

    async #updateFromPeers(getOpts: TGOptionsGet): Promise<void>
    {
        console.log('updateFromPeers');
        const peerNames = Array.from(this.peers.keys());

        if (peerNames.length)
        {
            await Promise.all(
                peerNames.map(name =>
                    this.#updateFromPeer(name, getOpts)
                )
            );
        }
    }

    async #updateFromPeer(peerName: string, getOpts: TGOptionsGet): Promise<void>
    {
        console.log('updateFromPeer');
        const soul = getOpts['#'];

        if (soul.startsWith(CHANGELOG_SOUL) || soul.startsWith(PEER_SYNC_SOUL))
        {
            return
        }

        const peerSoul  = createSoul('peers', peerName);
        const status    = await this.internal.get({
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
                        this.#updateChangelog(diff);
                    }

                    if (putToPeers)
                    {
                        const otherPeers = this.#getOtherPeers(peerName);
                        this.#updatePeers(diff, otherPeers);
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

        await this.internal.put({
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

    async #updateChangelog(diff: TGGraphData): Promise<void>
    {
        console.log('updateChangelog');
        const now  = new Date();
        const soul = createSoul(CHANGELOG_SOUL, now.getTime(), uuidv4());
        await this.internal.put({
            [soul]: {
                _: {
                    '#': soul,
                    '>': {
                        diff: now.getTime()
                    }
                },
                diff
            }
        });
    }

    #getOtherPeers(peerName: string): TGPeerMap
    {
        const peers = new Map();

        this.peers.forEach((value, key) =>
        {
            if (key !== peerName)
            {
                peers.set(key, value);
            }
        });

        return peers;
    };
}