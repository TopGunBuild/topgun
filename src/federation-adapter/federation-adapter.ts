import { TGChangeSetEntry, TGFederatedAdapterOptions, TGFederatedGraphAdapter, TGPeerSet } from './types';
import { TGGraphAdapter, TGGraphData, TGNode, TGOptionsGet } from '../types';
import { uuidv4 } from '../utils';
import { CHANGELOG_SOUL, DEFAULT_FEDERATION_OPTIONS, PEER_SYNC_SOUL } from './constants';
import { diffCRDT, mergeGraph } from '../crdt';

const NOOP = () =>
{
    // intentionally left blank
};

const getOtherPeers = (allPeers: TGPeerSet, peerName: string): TGPeerSet =>
{
    return Object.keys(allPeers).reduce((res, key) =>
    {
        if (key === peerName)
        {
            return res
        }
        return {
            ...res,
            [key]: allPeers[key]
        }
    }, {})
};

async function updateChangelog(internal: TGGraphAdapter, diff: TGGraphData): Promise<void>
{
    const now     = new Date();
    const itemKey = `${now.toISOString()}-${uuidv4()}`;

    await internal.put({
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

async function updateFromPeer(
    internal: TGGraphAdapter,
    persist: TGGraphAdapter,
    peerName: string,
    allPeers: TGPeerSet,
    getOpts: TGOptionsGet,
    adapterOpts?: TGFederatedAdapterOptions
): Promise<void>
{
    if (soul === CHANGELOG_SOUL || soul === PEER_SYNC_SOUL)
    {
        return
    }

    const peer       = allPeers[peerName];
    const otherPeers = getOtherPeers(allPeers, peerName);
    const {
        maxStaleness      = DEFAULT_FEDERATION_OPTIONS.maxStaleness,
        maintainChangelog = DEFAULT_FEDERATION_OPTIONS.maintainChangelog,
        putToPeers        = DEFAULT_FEDERATION_OPTIONS.putToPeers
    }          = adapterOpts || DEFAULT_FEDERATION_OPTIONS;
    const peerSoul   = `peers/${peerName}`;
    const now        = new Date().getTime();
    const status     = await internal.get({
        '#': `${peerSoul}/${soul}`,
    });
    const staleness  = now - ((status && status._['>'][soul]) || 0);

    if (staleness < maxStaleness)
    {
        return
    }

    const node = await peer.get({
        '#': soul
    });

    if (node)
    {
        try
        {
            const diff = await persist.put({
                [soul]: node
            });

            if (diff)
            {
                if (maintainChangelog)
                {
                    updateChangelog(internal, diff)
                }

                if (putToPeers)
                {
                    updatePeers(diff, otherPeers)
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

    await internal.put({
        [peerSoul]: {
            _: {
                '#': peerSoul,
                '>': {
                    [soul]: now
                }
            },
            [soul]: !!node
        }
    });
}

function updateFromPeers(
    internal: TGGraphAdapter,
    persist: TGGraphAdapter,
    allPeers: TGPeerSet,
    getOpts: TGOptionsGet,
    opts?: TGFederatedAdapterOptions
): Promise<void>
{
    const peerNames = Object.keys(allPeers);
    return peerNames.length
        ? Promise.all(
            peerNames.map(name =>
                updateFromPeer(internal, persist, name, allPeers, soul, opts)
            )
        ).then(NOOP)
        : Promise.resolve();
}

function updatePeers(data: TGGraphData, otherPeers: TGPeerSet): Promise<void>
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

export function getChangesetFeed(
    peer: TGGraphAdapter,
    from: string
): () => Promise<TGChangeSetEntry|null>
{
    let lastKey                                     = from;
    const changes: TGChangeSetEntry[]               = [];
    let nodePromise: Promise<TGGraphData|null>|null = null;

    return async function getNext(): Promise<readonly [string, TGGraphData]|null>
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

export async function syncWithPeer(
    internal: TGGraphAdapter,
    persist: TGGraphAdapter,
    peerName: string,
    allPeers: TGPeerSet,
    from: string,
    adapterOpts: TGFederatedAdapterOptions = DEFAULT_FEDERATION_OPTIONS
): Promise<string>
{
    const peer       = allPeers[peerName];
    const otherPeers = getOtherPeers(allPeers, peerName);
    const getNext    = getChangesetFeed(peer, from);
    let entry: TGChangeSetEntry|null;

    const batch = batchWriter(internal, persist, otherPeers, adapterOpts);

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

        await internal.put({
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

export function connectToPeer(
    internal: TGGraphAdapter,
    persist: TGGraphAdapter,
    allPeers: TGPeerSet,
    peerName: string,
    from: string,
    adapterOpts: TGFederatedAdapterOptions = DEFAULT_FEDERATION_OPTIONS
): () => void
{
    const peer       = allPeers[peerName];
    const otherPeers = getOtherPeers(allPeers, peerName);

    if (!peer || !peer.onChange)
    {
        throw new Error(`Unconnectable peer ${peerName}`);
    }

    const batch = batchWriter(internal, persist, otherPeers, adapterOpts);

    let disconnector: () => void;
    let batchTimeout: NodeJS.Timeout;
    (async () =>
    {
        // Catch up in batches before establishing connection
        let lastKey = await syncWithPeer(
            internal,
            persist,
            peerName,
            allPeers,
            from,
            adapterOpts
        );

        const { batchInterval = DEFAULT_FEDERATION_OPTIONS.batchInterval } = adapterOpts;

        let syncedKey = lastKey;

        async function writeBatch(): Promise<void>
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
            await internal.put({
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
        }

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

export function connectToPeers(
    internal: TGGraphAdapter,
    persist: TGGraphAdapter,
    allPeers: TGPeerSet,
    adapterOpts: TGFederatedAdapterOptions = DEFAULT_FEDERATION_OPTIONS
): () => void
{
    const { backSync = DEFAULT_FEDERATION_OPTIONS.backSync } = adapterOpts || DEFAULT_FEDERATION_OPTIONS;
    const peerNames                        = Object.keys(allPeers);
    const yesterday                        = new Date(Date.now() - backSync).toISOString();
    const connectable                      = peerNames.filter(
        peerName => !!(allPeers[peerName] && allPeers[peerName].onChange)
    );

    const disconnectors: Array<() => void> = [];

    connectable.map(async (peerName) =>
    {
        const node = await internal.get(PEER_SYNC_SOUL, { '.': peerName });
        const key  = (node && node[peerName]) || yesterday;
        disconnectors.push(
            connectToPeer(
                internal,
                persist,
                allPeers,
                peerName,
                key || yesterday,
                adapterOpts
            )
        )
    });

    return () => disconnectors.map(dc => dc());
}

export async function syncWithPeers(
    internal: TGGraphAdapter,
    persist: TGGraphAdapter,
    allPeers: TGPeerSet,
    adapterOpts: TGFederatedAdapterOptions = DEFAULT_FEDERATION_OPTIONS
): Promise<void>
{
    const { backSync = DEFAULT_FEDERATION_OPTIONS.backSync } = adapterOpts || DEFAULT_FEDERATION_OPTIONS;
    const peerNames                        = Object.keys(allPeers);
    const yesterday                        = new Date(Date.now() - backSync).toISOString();
    const unconnectable                    = peerNames.filter(
        peerName => !(allPeers[peerName] && allPeers[peerName].onChange)
    );

    return unconnectable.length
        ? Promise.all(
            unconnectable.map(async (peerName) =>
            {
                const node = await internal.get(PEER_SYNC_SOUL, { '.': peerName });
                const key  = (node && node[peerName]) || yesterday;

                return syncWithPeer(
                    internal,
                    persist,
                    peerName,
                    allPeers,
                    key,
                    adapterOpts
                )
            })
        ).then(NOOP)
        : Promise.resolve()
}

export function batchWriter(
    internal: TGGraphAdapter,
    persist: TGGraphAdapter,
    peers: TGPeerSet,
    adapterOpts: TGFederatedAdapterOptions = DEFAULT_FEDERATION_OPTIONS
): {
        readonly queueDiff: (changes: TGGraphData) => TGGraphData|undefined
        readonly writeBatch: () => Promise<TGGraphData|null>
    }
{
    const {
        maintainChangelog = DEFAULT_FEDERATION_OPTIONS.maintainChangelog,
        putToPeers        = DEFAULT_FEDERATION_OPTIONS.putToPeers
    } = adapterOpts || DEFAULT_FEDERATION_OPTIONS;

    let batch: TGGraphData = {};

    function queueDiff(changes: TGGraphData): TGGraphData|undefined
    {
        const diff = diffCRDT(changes, batch);
        batch      = diff ? mergeGraph(batch, diff, 'mutable') : batch;
        return diff
    }

    async function writeBatch(): Promise<TGGraphData|null>
    {
        if (!Object.keys(batch).length)
        {
            return null
        }
        const toWrite = batch;
        batch         = {};

        const diff = await persist.put(toWrite);

        if (diff)
        {
            if (maintainChangelog)
            {
                updateChangelog(internal, diff);
            }

            if (putToPeers)
            {
                updatePeers(diff, peers);
            }
        }

        return diff
    }

    return {
        queueDiff,
        writeBatch
    }
}

export function createFederatedAdapter(
    internal: TGGraphAdapter,
    external: TGPeerSet,
    persistence?: TGGraphAdapter,
    adapterOpts: TGFederatedAdapterOptions = DEFAULT_FEDERATION_OPTIONS
): TGFederatedGraphAdapter
{
    const {
        putToPeers        = DEFAULT_FEDERATION_OPTIONS.putToPeers,
        maintainChangelog = DEFAULT_FEDERATION_OPTIONS.maintainChangelog
    }       = adapterOpts;
    const persist = persistence || internal;
    const peers   = { ...external };

    return {
        get: async (getOpts: TGOptionsGet) =>
        {
            await updateFromPeers(internal, persist, peers, getOpts, adapterOpts);
            return internal.get(getOpts);
        },

        put: async (data: TGGraphData) =>
        {
            const diff = await persist.put(data);

            if (!diff)
            {
                return diff
            }

            if (maintainChangelog)
            {
                updateChangelog(internal, diff)
            }

            if (putToPeers)
            {
                updatePeers(diff, peers)
            }

            return diff
        },

        syncWithPeers: () => syncWithPeers(internal, persist, external, adapterOpts),

        connectToPeers: () => connectToPeers(internal, persist, external, adapterOpts),

        getChangesetFeed: (from: string) => getChangesetFeed(internal, from)
    }
}

export const FederationAdapter = {
    create: createFederatedAdapter
};