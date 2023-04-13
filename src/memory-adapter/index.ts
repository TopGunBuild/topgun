import { diffCRDT, mergeGraph } from '../crdt';
import { TGGraphAdapter, TGGraphData, TGNode } from '../types';
import { cloneValue } from '../utils/clone-value';

const DEFAULT_OPTS = {
    diffFn: diffCRDT,
    mergeFn: mergeGraph,
};

interface MemoryAdapterOpts {
    readonly diffFn?: typeof diffCRDT;
    readonly mergeFn?: typeof mergeGraph;
    readonly direct?: boolean;
}

const getSync = (
    opts: MemoryAdapterOpts,
    graph: TGGraphData,
    soul: string,
): TGNode | null =>
    (opts.direct ? graph[soul] : cloneValue(graph[soul])) || null;

const get = (
    opts: MemoryAdapterOpts,
    graph: TGGraphData,
    soul: string,
): Promise<TGNode | null> => Promise.resolve(getSync(opts, graph, soul));

const putSync = (
    {
        diffFn = DEFAULT_OPTS.diffFn,
        mergeFn = DEFAULT_OPTS.mergeFn,
    }: MemoryAdapterOpts,
    graph: TGGraphData,
    graphData: TGGraphData,
) => 
{
    const diff = diffFn(graphData, graph);

    if (diff) 
    {
        mergeFn(graph, diff, 'mutable');
    }

    return diff || null;
};

const put = (
    opts: MemoryAdapterOpts,
    graph: TGGraphData,
    graphData: TGGraphData,
): Promise<TGGraphData | null> =>
    Promise.resolve(putSync(opts, graph, graphData));

export function createMemoryAdapter(
    opts: MemoryAdapterOpts = DEFAULT_OPTS,
): TGGraphAdapter 
{
    const graph: TGGraphData = {};

    return {
        get: (soul: string) => get(opts, graph, soul),
        getSync: (soul: string) => getSync(opts, graph, soul),
        put: (graphData: TGGraphData) => put(opts, graph, graphData),
        putSync: (graphData: TGGraphData) => putSync(opts, graph, graphData),
    };
}
