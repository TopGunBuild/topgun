import { isNumber, isString } from '@topgunbuild/typed';
import { TGQueryOptions, TGStorage } from './types';
import { TGGraphAdapter, TGGraphAdapterOptions, TGGraphData, TGOptionsGet } from '../types';
import { diffCRDT, mergeGraph } from '../crdt';
import { assertPutEntry, queryOptionsFromGetOptions } from './utils';

const DEFAULT_CRDT_OPTS = {
    diffFn : diffCRDT,
    mergeFn: mergeGraph,
};

export function createGraphAdapter(storage: TGStorage, adapterOptions?: TGGraphAdapterOptions): TGGraphAdapter
{
    return {
        get: (opts: TGOptionsGet) => get(storage, opts),
        put: (graphData: TGGraphData) => put(storage, graphData, adapterOptions),
    };
}

async function get(
    db: TGStorage,
    opts: TGOptionsGet
): Promise<TGGraphData>
{
    const listOptions = queryOptionsFromGetOptions(opts);

    if (listOptions)
    {
        return await getList(db, listOptions);
    }

    const soul = opts['#'];

    if (isString(soul))
    {
        return {
            [soul]: (await db.get(soul) || null)
        };
    }

    return {};
}

async function getList(db: TGStorage, options: TGQueryOptions): Promise<TGGraphData>
{
    if (isNumber(options.limit) && options.limit <= 0)
    {
        throw new TypeError('List limit must be positive.');
    }
    if (isNumber(options.start))
    {
        if (isNumber(options.limit))
        {
            options.limit++;
        }
    }

    return await db.list(options);
}

async function put(
    db: TGStorage,
    data: TGGraphData,
    adapterOptions?: TGGraphAdapterOptions,
    crdtOptions = DEFAULT_CRDT_OPTS,
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
            adapterOptions,
            crdtOptions,
        );

        if (nodeDiff)
        {
            diff[soul] = nodeDiff[soul];
        }
    }

    return Object.keys(diff).length ? diff : null;
}

async function patchGraphFull(
    db: TGStorage,
    data: TGGraphData,
    adapterOptions?: TGGraphAdapterOptions,
    crdtOptions = DEFAULT_CRDT_OPTS,
): Promise<TGGraphData|null>
{
    while (true)
    {
        const patchDiffData = await getPatchDiff(db, data, crdtOptions);

        if (!patchDiffData)
        {
            return null;
        }
        const { diff, toWrite } = patchDiffData;

        if (await writeRawGraph(db, toWrite, adapterOptions))
        {
            return diff;
        }

        console.warn('unsuccessful patch, retrying', Object.keys(diff));
    }
}

async function getPatchDiff(
    db: TGStorage,
    data: TGGraphData,
    crdtOptions = DEFAULT_CRDT_OPTS,
): Promise<null|{
        readonly diff: TGGraphData;
        readonly existing: TGGraphData;
        readonly toWrite: TGGraphData;
    }>
{
    const { diffFn = diffCRDT, mergeFn = mergeGraph } = crdtOptions;
    const existing                                    = await getExisting(db, data);
    const graphDiff                                   = diffFn(data, existing);

    if (!graphDiff || !Object.keys(graphDiff).length)
    {
        return null;
    }

    const existingFromDiff: TGGraphData = {};

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

async function getExisting(
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

async function writeRawGraph(
    db: TGStorage,
    data: TGGraphData,
    adapterOptions?: TGGraphAdapterOptions
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

            assertPutEntry(soul, nodeToWrite, adapterOptions);
            await db.put(soul, nodeToWrite);
        }

        return true;
    }
    catch (e)
    {
        throw e;
    }
}
