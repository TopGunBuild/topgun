import {
    CRDTOpts,
    GraphData,
    Node,
    NodeState,
    PartialGraphData
} from '../types'
import { cloneValue } from '../utils/clone-value';

const EMPTY: any = {};

export function addMissingState(graphData: PartialGraphData): GraphData
{
    const updatedGraphData = cloneValue(graphData);
    const now              = new Date().getTime();

    for (const soul in graphData)
    {
        if (!soul)
        {
            continue
        }

        const node = graphData[soul];
        if (!node)
        {
            continue
        }
        const meta  = (node._ = node._ || {});
        meta['#']   = soul;
        const state = (meta['>'] = meta['>'] || {});

        for (const key in node)
        {
            if (key === '_')
            {
                continue
            }
            state[key] = state[key] || now
        }

        updatedGraphData[soul] = node
    }

    return updatedGraphData as GraphData;
}

const DEFAULT_OPTS = {
    Lexical    : JSON.stringify,
    futureGrace: 10 * 60 * 1000
};

export function diffCRDT(
    updatedGraph: GraphData,
    existingGraph: GraphData,
    opts: CRDTOpts = DEFAULT_OPTS
): GraphData|undefined
{
    const {
              machineState = new Date().getTime(),
              futureGrace  = DEFAULT_OPTS.futureGrace,
              Lexical      = DEFAULT_OPTS.Lexical
          }        = opts || EMPTY;
    const maxState = machineState + futureGrace;

    const allUpdates: GraphData = {};

    /*console.log({
        updatedGraph: JSON.stringify(updatedGraph),
        existingGraph: JSON.stringify(existingGraph)
    });*/

    for (const soul in updatedGraph)
    {
        if (!soul)
        {
            continue
        }
        const existing                 = existingGraph[soul];
        const updated                  = updatedGraph[soul];
        const existingState: NodeState = (existing && existing._ && existing._['>']) || EMPTY;
        const updatedState: NodeState  = (updated && updated._ && updated._['>']) || EMPTY;

        if (!updated)
        {
            if (!(soul in existingGraph))
            {
                allUpdates[soul] = updated;
            }
            continue
        }

        let hasUpdates = false;

        const updates: Node = {
            _: {
                '#': soul,
                '>': {}
            }
        };

        for (const key in updatedState)
        {
            if (!key)
            {
                continue
            }

            const existingKeyState = existingState[key];
            const updatedKeyState  = updatedState[key];

            if (updatedKeyState > maxState || !updatedKeyState)
            {
                continue
            }
            if (existingKeyState && existingKeyState >= updatedKeyState)
            {
                continue
            }
            if (existingKeyState === updatedKeyState)
            {
                const existingVal = (existing && existing[key]) || undefined;
                const updatedVal  = updated[key];
                // This is based on TopGun logic
                if (Lexical(updatedVal) <= Lexical(existingVal))
                {
                    continue
                }
            }
            updates[key]        = updated[key];
            updates._['>'][key] = updatedKeyState;
            hasUpdates          = true
        }

        if (hasUpdates)
        {
            allUpdates[soul] = updates;
        }
    }

    return Object.keys(allUpdates) ? allUpdates : undefined
}

export function mergeNodes(
    existing: Node|undefined,
    updates: Node|undefined,
    mut: 'immutable'|'mutable' = 'immutable'
): Node|undefined
{
    if (!existing)
    {
        return updates
    }
    if (!updates)
    {
        return existing
    }
    const existingMeta  = existing._ || {};
    const existingState = existingMeta['>'] || {};
    const updatedMeta   = updates._ || {};
    const updatedState  = updatedMeta['>'] || {};

    if (mut === 'mutable')
    {
        existingMeta['>'] = existingState;
        existing._        = existingMeta;

        for (const key in updatedState)
        {
            if (!key)
            {
                continue
            }
            existing[key]      = updates[key];
            existingState[key] = updatedState[key]
        }

        return existing
    }

    return {
        ...existing,
        ...updates,
        _: {
            '#': existingMeta['#'],
            '>': {
                ...existingMeta['>'],
                ...updates._['>']
            }
        }
    }
}

export function mergeGraph(
    existing: GraphData,
    diff: GraphData,
    mut: 'immutable'|'mutable' = 'immutable'
): GraphData
{
    const result: GraphData = mut ? existing : { ...existing };

    for (const soul in diff)
    {
        if (!soul)
        {
            continue
        }
        result[soul] = mergeNodes(existing[soul], diff[soul], mut)
    }
    return result
}

