import {
    CRDTOptions,
    TGGraphData,
    TGNode,
    TGNodeMeta,
    TGNodeState,
} from '../types';
import { cloneValue } from '@topgunbuild/typed';

const EMPTY: any = {};

export function addMissingState(graphData: Partial<TGGraphData>): TGGraphData
{
    const updatedGraphData = cloneValue(graphData);
    const now              = new Date().getTime();

    for (const soul in graphData)
    {
        if (!soul)
        {
            continue;
        }

        const node = graphData[soul];
        if (!node)
        {
            continue;
        }
        const meta  = (node._ = node._ || { '#': null, '>': {} });
        meta['#']   = soul;
        const state = (meta['>'] = meta['>'] || {});

        for (const key in node)
        {
            if (key === '_')
            {
                continue;
            }
            state[key] = state[key] || now;
        }

        updatedGraphData[soul] = node;
    }

    return updatedGraphData as TGGraphData;
}

const DEFAULT_OPTS = {
    Lexical    : JSON.stringify,
    futureGrace: 10 * 60 * 1000,
};

export function diffCRDT(
    updatedGraph: TGGraphData,
    existingGraph: TGGraphData,
    opts: CRDTOptions = DEFAULT_OPTS,
): TGGraphData|undefined
{
    const {
        machineState = new Date().getTime(),
        futureGrace  = DEFAULT_OPTS.futureGrace,
        Lexical      = DEFAULT_OPTS.Lexical,
    }        = opts || EMPTY;
    const maxState = machineState + futureGrace;

    const allUpdates: TGGraphData = {};

    for (const soul in updatedGraph)
    {
        if (!soul)
        {
            continue;
        }
        const existing                   = existingGraph[soul];
        const updated                    = updatedGraph[soul];
        const existingState: TGNodeState =
                  (existing && existing._ && existing._['>']) || EMPTY;
        const updatedState: TGNodeState  =
                  (updated && updated._ && updated._['>']) || EMPTY;

        if (!updated)
        {
            allUpdates[soul] = updated;
            continue;
        }

        let hasUpdates = false;

        const updates: TGNode = {
            _: {
                '#': soul,
                '>': {},
            },
        };

        for (const key in updatedState)
        {
            if (!key)
            {
                continue;
            }

            const existingKeyState = existingState[key];
            const updatedKeyState  = updatedState[key];

            if (updatedKeyState > maxState || !updatedKeyState)
            {
                continue;
            }
            if (existingKeyState && existingKeyState >= updatedKeyState)
            {
                continue;
            }
            if (existingKeyState === updatedKeyState)
            {
                const existingVal = (existing && existing[key]) || undefined;
                const updatedVal  = updated[key];
                // This is based on TopGun logic
                if (Lexical(updatedVal) <= Lexical(existingVal))
                {
                    continue;
                }
            }
            updates[key] = updated[key];

            if (updates._ && updates._['>'])
            {
                updates._['>'][key] = updatedKeyState;
            }

            hasUpdates = true;
        }

        if (hasUpdates)
        {
            allUpdates[soul] = updates;
        }
    }

    return Object.keys(allUpdates).length > 0 ? allUpdates : undefined;
}

export function mergeNodes(
    existing: TGNode|undefined,
    updates: TGNode|undefined,
    mut: 'immutable'|'mutable' = 'immutable',
): TGNode|undefined
{
    if (!existing)
    {
        return updates;
    }
    if (updates === null)
    {
        return null;
    }
    else if (!updates)
    {
        return existing;
    }
    const existingMeta  = existing._ || {};
    const existingState = existingMeta['>'] || {};
    const updatedMeta   = updates._ || {};
    const updatedState  = updatedMeta['>'] || {};

    if (mut === 'mutable')
    {
        for (const key in updatedState)
        {
            if (!key)
            {
                continue;
            }
            existing[key]      = updates[key];
            existingState[key] = updatedState[key];
        }

        existingMeta['>'] = existingState;
        existing._        = existingMeta as TGNodeMeta;

        return existing;
    }

    return {
        ...existing,
        ...updates,
        _: {
            '#': existingMeta['#'],
            '>': {
                ...existingMeta['>'],
                ...((updates._ && updates._['>']) || {}),
            },
        },
    };
}

export function mergeGraph(
    existing: TGGraphData,
    diff: TGGraphData,
    mut: 'immutable'|'mutable' = 'immutable',
): TGGraphData
{
    const result: TGGraphData = mut ? existing : { ...existing };

    for (const soul in diff)
    {
        if (!soul)
        {
            continue;
        }
        result[soul] = mergeNodes(existing[soul], diff[soul], mut);
    }
    return result;
}
