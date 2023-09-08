import { TGChangeSetEntry, TGFederatedAdapterOptions, TGPeerSet } from './types';
import { TGGraphAdapter, TGGraphData, TGOptionsGet } from '../types';
import { uuidv4 } from '../utils';
import { CHANGELOG_SOUL, DEFAULT_FEDERATION_OPTIONS, PEER_SYNC_SOUL } from './constants';
import { diffCRDT, mergeGraph } from '../crdt';
import { NOOP } from '../utils/noop';

export class TGFederationAdapter implements TGGraphAdapter
{
    internal: TGGraphAdapter;
    peers: TGPeerSet;
    persistence: TGGraphAdapter;
    adapterOpts: TGFederatedAdapterOptions;
    putToPeers: boolean;
    maintainChangelog: boolean;

    /**
     * Constructor
     */
    constructor(
        internal: TGGraphAdapter,
        peers: TGPeerSet,
        persistence: TGGraphAdapter,
        adapterOpts: TGFederatedAdapterOptions = DEFAULT_FEDERATION_OPTIONS
    )
    {
        this.internal          = internal;
        this.peers             = { ...peers };
        this.persistence       = persistence;
        this.adapterOpts       = adapterOpts;
        this.putToPeers        = adapterOpts.putToPeers || DEFAULT_FEDERATION_OPTIONS.putToPeers;
        this.maintainChangelog = adapterOpts.maintainChangelog || DEFAULT_FEDERATION_OPTIONS.maintainChangelog;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async get(getOpts: TGOptionsGet)
    {
        await this.#updateFromPeers(this.peers, getOpts);
        return this.internal.get(getOpts);
    }

    async put(data: TGGraphData)
    {
        const diff = await this.persistence.put(data);

        if (!diff)
        {
            return diff
        }

        if (this.maintainChangelog)
        {
            this.#updateChangelog(diff);
        }

        if (this.putToPeers)
        {
            this.#updatePeers(diff, this.peers);
        }

        return diff
    }

    async syncWithPeer(peerName: string, from: string): Promise<string>
    {
        const peer       = this.peers[peerName];
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

            await this.internal.put({
                [PEER_SYNC_SOUL]: {
                    _: {
                        '#': PEER_SYNC_SOUL,
                        '>': {
                            [peerName]: new Date().getTime()
                        }
                    },
                    [peerName]: lastSeenKey
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
        const peerNames   = Object.keys(this.peers);
        const connectable = peerNames.filter(
            peerName => !!(this.peers[peerName] && this.peers[peerName].onChange)
        );

        const disconnectors: Array<() => void> = [];

        connectable.map(async (peerName) =>
        {
            const key = await this.#getPeerSyncDate(peerName);

            disconnectors.push(
                this.#connectToPeer(peerName, key)
            )
        });

        return () => disconnectors.map(dc => dc());
    }

    syncWithPeers(): Promise<void>
    {
        const peerNames     = Object.keys(this.peers);
        const unconnectable = peerNames.filter(
            peerName => !(this.peers[peerName] && this.peers[peerName].onChange)
        );

        return unconnectable.length
            ? Promise.all(
                unconnectable.map(async (peerName) =>
                {
                    const key = await this.#getPeerSyncDate(peerName);

                    return this.syncWithPeer(peerName, key);
                })
            ).then(NOOP)
            : Promise.resolve()
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #getPeerSyncDate(peerName: string): Promise<string>
    {
        const { backSync = DEFAULT_FEDERATION_OPTIONS.backSync } = this.adapterOpts || DEFAULT_FEDERATION_OPTIONS;

        const yesterday = new Date(Date.now() - backSync).toISOString();
        const graph     = await this.internal.get({
            '#': PEER_SYNC_SOUL
        });

        return (graph[PEER_SYNC_SOUL] && graph[PEER_SYNC_SOUL][peerName]) || yesterday;
    }

    #connectToPeer(peerName: string, from: string): () => void
    {
        const peer       = this.peers[peerName];
        const otherPeers = this.#getOtherPeers(peerName);

        if (!peer || !peer.onChange)
        {
            throw new Error(`Unconnectable peer ${peerName}`);
        }

        const batch = this.#batchWriter(otherPeers);

        let disconnector: () => void;
        let batchTimeout: NodeJS.Timeout;
        (async () =>
        {
            // Catch up in batches before establishing connection
            let lastKey = await this.syncWithPeer(peerName, from);

            const { batchInterval = DEFAULT_FEDERATION_OPTIONS.batchInterval } = this.adapterOpts;

            let syncedKey = lastKey;

            const writeBatch = async (): Promise<void> =>
            {
                if (syncedKey === lastKey)
                {
                    if (batchInterval)
                    {
                        batchTimeout = setTimeout(writeBatch, batchInterval)
                    }
                    return
                }

                syncedKey = lastKey;

                await batch.writeBatch();
                await this.internal.put({
                    [PEER_SYNC_SOUL]: {
                        _: {
                            '#': PEER_SYNC_SOUL,
                            '>': {
                                [peerName]: new Date().getTime()
                            }
                        },
                        [peerName]: lastKey
                    }
                });

                if (batchInterval)
                {
                    batchTimeout = setTimeout(writeBatch, batchInterval)
                }
            };

            disconnector = peer.onChange!(([key, changes]) =>
            {
                try
                {
                    batch.queueDiff(changes);
                    lastKey = key;
                    if (!batchInterval)
                    {
                        writeBatch()
                    }
                }
                catch (e: any)
                {
                    console.warn('Error syncing from peer', peerName, e.stack)
                }
            }, lastKey);

            if (batchInterval)
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
        let lastKey                                     = from;
        const changes: TGChangeSetEntry[]               = [];
        let nodePromise: Promise<TGGraphData|null>|null = null;

        return async (): Promise<readonly [string, TGGraphData]|null> =>
        {
            if (!changes.length && !nodePromise)
            {
                nodePromise = peer.get({
                    '#': CHANGELOG_SOUL,
                    '.': {
                        '>': `${lastKey}一`
                    }
                });
                const node  = await nodePromise;
                nodePromise = null;

                if (node)
                {
                    for (const key in node)
                    {
                        if (key && key !== '_')
                        {
                            changes.splice(0, 0, [key, node[key]]);
                            lastKey = key
                        }
                    }
                }
            }
            else if (nodePromise)
            {
                await nodePromise;
                nodePromise = null
            }

            const entry = changes.pop();
            return entry || null
        }
    }

    #batchWriter(peers: TGPeerSet): {
        readonly queueDiff: (changes: TGGraphData) => TGGraphData|undefined
        readonly writeBatch: () => Promise<TGGraphData|null>
    }
    {
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
                if (this.maintainChangelog)
                {
                    this.#updateChangelog(diff);
                }

                if (this.putToPeers)
                {
                    this.#updatePeers(diff, peers);
                }
            }

            return diff;
        };

        return { queueDiff, writeBatch };
    }

    #updatePeers(data: TGGraphData, otherPeers: TGPeerSet): Promise<void>
    {
        const entries = Object.entries(otherPeers);
        return entries.length
            ? Promise.all(
                entries.map(([name, peer]) =>
                    peer.put(data).catch((err) =>
                    {
                        console.warn('Failed to update peer', name, err.stack || err, data)
                    })
                )
            ).then(NOOP)
            : Promise.resolve();
    }

    async #updateFromPeers(allPeers: TGPeerSet, getOpts: TGOptionsGet): Promise<void>
    {
        const peerNames = Object.keys(allPeers);
        return peerNames.length
            ? Promise.all(
                peerNames.map(name =>
                    this.#updateFromPeer(name, getOpts)
                )
            ).then(NOOP)
            : Promise.resolve();
    }

    async #updateFromPeer(peerName: string, getOpts: TGOptionsGet): Promise<void>
    {
        const soul = getOpts['#'];

        if (soul === CHANGELOG_SOUL || soul === PEER_SYNC_SOUL)
        {
            return
        }

        const peer       = this.peers[peerName];
        const otherPeers = this.#getOtherPeers(peerName);
        const {
            maxStaleness      = DEFAULT_FEDERATION_OPTIONS.maxStaleness,
            maintainChangelog = DEFAULT_FEDERATION_OPTIONS.maintainChangelog,
            putToPeers        = DEFAULT_FEDERATION_OPTIONS.putToPeers
        }          = this.adapterOpts;
        const peerSoul   = `peers/${peerName}`;
        const now        = new Date().getTime();
        const status     = await this.internal.get({
            '#': `${peerSoul}/${soul}`,
        });
        const staleness  = now - ((status && status._['>'][soul]) || 0);

        if (staleness < maxStaleness)
        {
            return
        }

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
        const now     = new Date();
        const itemKey = `${now.toISOString()}-${uuidv4()}`;

        await this.internal.put({
            [CHANGELOG_SOUL]: {
                _: {
                    '#': CHANGELOG_SOUL,
                    '>': {
                        [itemKey]: now.getTime()
                    }
                },
                [itemKey]: diff
            }
        })
    }

    #getOtherPeers(peerName: string): TGPeerSet
    {
        return Object.keys(this.peers).reduce((res, key) =>
        {
            if (key === peerName)
            {
                return res
            }
            return { ...res, [key]: this.peers[key] }
        }, {})
    };
}