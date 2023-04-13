import { diffCRDT, mergeGraph } from '../crdt';
import { TGGraphAdapter, TGGraphData, TGNode } from '../types';
import { IndexedDb } from './indexeddb';

const DEFAULT_DB_NAME = 'topgun-nodes';

type RawWideNodeData = Record<string, any>;
type RawNodeData = null | RawWideNodeData;
type RawGraphData = Record<string, RawNodeData>;

export const DEFAULT_CRDT_OPTS = {
    diffFn: diffCRDT,
    mergeFn: mergeGraph,
};

export function createGraphAdapter(name = DEFAULT_DB_NAME): TGGraphAdapter 
{
    const db = new IndexedDb(name);
    return adapterFromIndexedDB(db);
}

export function adapterFromIndexedDB(db: IndexedDb): TGGraphAdapter 
{
    return {
        get: (soul: string) => getNode(db, soul),
        put: (graphData: TGGraphData) => patchGraph(db, graphData),
    };
}

export async function getNode(
    db: IndexedDb,
    soul: string,
): Promise<TGNode | null> 
{
    return db.get<TGNode>(soul);
}

export async function patchGraph(
    db: IndexedDb,
    data: TGGraphData,
    opts = DEFAULT_CRDT_OPTS,
): Promise<TGGraphData | null> 
{
    const diff: any = {};

    for (const soul in data) 
    {
        if (!soul) 
        {
            continue;
        }

        const nodeDiff = await patchGraphFull(
            db,
            {
                [soul]: data[soul],
            },
            opts,
        );

        if (nodeDiff) 
        {
            diff[soul] = nodeDiff[soul];
        }
    }

    return Object.keys(diff).length ? diff : null;
}

export async function patchGraphFull(
    db: IndexedDb,
    data: TGGraphData,
    opts = DEFAULT_CRDT_OPTS,
): Promise<TGGraphData | null> 
{
    while (true) 
    {
        const patchDiffData = await getPatchDiff(db, data, opts);

        if (!patchDiffData) 
        {
            return null;
        }
        const { diff, toWrite } = patchDiffData;

        if (await writeRawGraph(db, toWrite)) 
        {
            return diff;
        }

        console.warn('unsuccessful patch, retrying', Object.keys(diff));
    }
}

export async function getPatchDiff(
    db: IndexedDb,
    data: TGGraphData,
    opts = DEFAULT_CRDT_OPTS,
): Promise<null | {
    readonly diff: TGGraphData;
    readonly existing: RawGraphData;
    readonly toWrite: RawGraphData;
}> 
{
    const { diffFn = diffCRDT, mergeFn = mergeGraph } = opts;
    const existing = await getExisting(db, data);
    const graphDiff = diffFn(data, existing);

    if (!graphDiff || !Object.keys(graphDiff).length) 
    {
        return null;
    }

    const existingFromDiff: any = {};

    for (const soul in graphDiff) 
    {
        if (!soul) 
        {
            continue;
        }

        existingFromDiff[soul] = existing[soul];
    }

    const updatedGraph = mergeFn(existing, graphDiff, 'mutable');

    return {
        diff: graphDiff,
        existing: existingFromDiff,
        toWrite: updatedGraph as RawGraphData,
    };
}

export async function getExisting(
    db: IndexedDb,
    data: TGGraphData,
): Promise<TGGraphData> 
{
    const existingData: TGGraphData = {};

    for (const soul in data) 
    {
        if (!soul) 
        {
            continue;
        }

        existingData[soul] = await db.get<TGNode>(soul);
    }

    return existingData;
}

export async function writeRawGraph(
    db: IndexedDb,
    data: RawGraphData,
): Promise<boolean> 
{
    try 
    {
        for (const soul in data) 
        {
            if (!soul) 
            {
                continue;
            }

            const nodeToWrite = data[soul];

            if (!nodeToWrite) 
            {
                // TODO db.removeItem(soul)?
                continue;
            }

            await db.set(soul, nodeToWrite);
        }

        return true;
    }
    catch (e) 
    {
        throw e;
    }
}
