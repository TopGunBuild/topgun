import { TGStorage } from './types';
import { TGGraphData, TGNode, TGOptionsGet } from '../types';
import { diffCRDT, mergeGraph } from '../crdt';
import { DEFAULT_CRDT_OPTS } from '../indexeddb/indexeddb-adapter';

export async function getNodes(
    db: TGStorage,
    soul: string,
    opts?: TGOptionsGet
): Promise<TGNode|null>
{
    return db.get<TGNode>(soul);
}

export async function patchGraph(
    db: TGStorage,
    data: TGGraphData,
    opts = DEFAULT_CRDT_OPTS,
): Promise<TGGraphData|null>
{
    const diff: TGGraphData = {};

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
    db: TGStorage,
    data: TGGraphData,
    opts = DEFAULT_CRDT_OPTS,
): Promise<TGGraphData|null>
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
    db: TGStorage,
    data: TGGraphData,
    opts = DEFAULT_CRDT_OPTS,
): Promise<null|{
        readonly diff: TGGraphData;
        readonly existing: TGGraphData;
        readonly toWrite: TGGraphData;
    }>
{
    const { diffFn = diffCRDT, mergeFn = mergeGraph } = opts;
    const existing                                    = await getExisting(db, data);
    const graphDiff                                   = diffFn(data, existing);

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
        diff    : graphDiff,
        existing: existingFromDiff,
        toWrite : updatedGraph as TGGraphData,
    };
}

export async function getExisting(
    db: TGStorage,
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

        existingData[soul] = await db.get(soul);
    }

    return existingData;
}

export async function writeRawGraph(
    db: TGStorage,
    data: TGGraphData,
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

            await db.put(soul, nodeToWrite);
        }

        return true;
    }
    catch (e)
    {
        throw e;
    }
}
