import { diffCRDT, mergeGraph } from '../crdt';
import { OptionsGet, GraphAdapter, GraphData, Node } from '../types';
import { IndexedDb } from './indexeddb';

const DEFAULT_DB_NAME = 'topgun-nodes';

type RawWideNodeData = Record<string, any>
type RawNodeData = null|RawWideNodeData
type RawGraphData = Record<string, RawNodeData>

export const DEFAULT_CRDT_OPTS = {
    diffFn : diffCRDT,
    mergeFn: mergeGraph
};

/**
 * Open a IndexedDB database as a Graph Adapter
 */
export function createGraphAdapter(
    name = DEFAULT_DB_NAME
): GraphAdapter
{
    const db = new IndexedDb(name);
    return adapterFromIndexedDB(db);
}

/**
 * Create Graph Adapter from IndexedDB database
 */
export function adapterFromIndexedDB(
    db: IndexedDb
): GraphAdapter
{
    return {
        get: (soul: string, opts?: OptionsGet) => getNode(db, soul, opts),
        put: (graphData: GraphData) => patchGraph(db, graphData)
    };
}

export async function getNode(
    db: IndexedDb,
    soul: string,
    opts?: OptionsGet
): Promise<Node|null>
{
    return db.get<Node>(soul);
}

export async function patchGraph(
    db: IndexedDb,
    data: GraphData,
    opts = DEFAULT_CRDT_OPTS
): Promise<GraphData|null>
{
    const diff: any = {};

    for (const soul in data)
    {
        if (!soul)
        {
            continue
        }

        const nodeDiff = await patchGraphFull(
            db,
            {
                [soul]: data[soul]
            },
            opts
        );

        if (nodeDiff)
        {
            diff[soul] = nodeDiff[soul];
        }
    }

    return Object.keys(diff).length ? diff : null
}

export async function patchGraphFull(
    db: IndexedDb,
    data: GraphData,
    opts = DEFAULT_CRDT_OPTS
): Promise<GraphData|null>
{
    while (true)
    {
        const patchDiffData = await getPatchDiff(db, data, opts);

        if (!patchDiffData)
        {
            return null;
        }
        const { diff, existing, toWrite } = patchDiffData;

        if (await writeRawGraph(db, toWrite, existing))
        {
            return diff
        }

        console.warn('unsuccessful patch, retrying', Object.keys(diff));
    }
}

export async function getPatchDiff(
    db: IndexedDb,
    data: GraphData,
    opts = DEFAULT_CRDT_OPTS
): Promise<null|{
    readonly diff: GraphData
    readonly existing: RawGraphData
    readonly toWrite: RawGraphData
}>
{
    const { diffFn = diffCRDT, mergeFn = mergeGraph } = opts;
    const existing                                    = await getExisting(db, data);
    const graphDiff                                   = diffFn(data, existing);

    if (!graphDiff || !Object.keys(graphDiff).length)
    {
        return null
    }

    const existingFromDiff: any = {};

    for (const soul in graphDiff)
    {
        if (!soul)
        {
            continue
        }

        existingFromDiff[soul] = existing[soul];
    }

    const updatedGraph = mergeFn(existing, graphDiff, 'mutable');

    return {
        diff    : graphDiff,
        existing: existingFromDiff,
        toWrite : updatedGraph
    };
}

export async function getExisting(
    db: IndexedDb,
    data: GraphData
): Promise<GraphData>
{
    const existingData: GraphData = {};

    for (const soul in data)
    {
        if (!soul)
        {
            continue;
        }

        existingData[soul] = await db.get<Node>(soul);
    }

    return existingData;
}

export async function writeRawGraph(
    db: IndexedDb,
    data: RawGraphData,
    existing: RawGraphData
): Promise<boolean>
{
    try
    {
        for (const soul in data)
        {
            if (!soul)
            {
                continue
            }

            const nodeToWrite = data[soul];

            if (!nodeToWrite)
            {
                // TODO db.removeItem(soul)?
                continue
            }

            await db.set(soul, nodeToWrite);
        }

        return true;
    }
    catch (e)
    {
        throw e
    }
}