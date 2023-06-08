import { isNumber, isString } from 'topgun-typed';
import { StorageListOptions, TGStorage } from './types';
import { LEX, TGGraphAdapter, TGGraphData, TGOptionsGet } from '../types';
import { diffCRDT, mergeGraph } from '../crdt';
import { DEFAULT_CRDT_OPTS } from '../indexeddb/indexeddb-adapter';
import { assertPutEntry } from './utils';

export function createGraphAdapter(storage: TGStorage): TGGraphAdapter
{
    return {
        get: (soul: string, opts?: TGOptionsGet) => get(storage, soul, opts),
        put: (graphData: TGGraphData) => put(storage, graphData),
    };
}

async function get(
    db: TGStorage,
    soul: string,
    opts?: TGOptionsGet
): Promise<TGGraphData>
{
    const lexQuery: LEX|undefined  = opts && opts['.'];
    const limit: number|undefined  = opts && opts['%'];
    const prefix: string|undefined = lexQuery && lexQuery['*'];
    const start: string|undefined  = lexQuery && lexQuery['>'];
    const end: string|undefined    = lexQuery && lexQuery['<'];

    soul = soul || (opts && opts['#']);

    if (isString(prefix) || isString(start) || isString(end))
    {
        const options: StorageListOptions = {};
        const getPath                     = (path: string) => [soul, path].join('/');

        if (start)
        {
            options.start = getPath(start);
        }
        if (end)
        {
            options.end = getPath(end);
        }
        if (prefix)
        {
            options.prefix = getPath(prefix);
        }
        if (limit)
        {
            options.limit = limit;
        }

        return await getList(db, options);
    }

    if (isString(soul))
    {
        return {
            [soul]: (await db.get(soul) || null)
        };
    }

    return {};
}

async function getList(db: TGStorage, options: StorageListOptions): Promise<TGGraphData>
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

async function patchGraphFull(
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

async function getPatchDiff(
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

            assertPutEntry(soul, nodeToWrite);
            await db.put(soul, nodeToWrite);
        }

        return true;
    }
    catch (e)
    {
        throw e;
    }
}
