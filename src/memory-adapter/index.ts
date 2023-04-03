import { diffCRDT, mergeGraph } from '../crdt'
import { GraphAdapter, GraphData, Node } from '../types'
import { cloneValue } from '../utils/clone-value';

const DEFAULT_OPTS = {
    diffFn : diffCRDT,
    mergeFn: mergeGraph
};

interface MemoryAdapterOpts
{
    readonly diffFn?: typeof diffCRDT
    readonly mergeFn?: typeof mergeGraph
    readonly direct?: boolean
}

const getSync = (
    opts: MemoryAdapterOpts,
    graph: GraphData,
    soul: string
): Node|null => (opts.direct ? graph[soul] : cloneValue(graph[soul])) || null;

const get = (
    opts: MemoryAdapterOpts,
    graph: GraphData,
    soul: string
): Promise<Node|null> => Promise.resolve(getSync(opts, graph, soul));

const putSync = (
    {
        diffFn = DEFAULT_OPTS.diffFn,
        mergeFn = DEFAULT_OPTS.mergeFn
    }: MemoryAdapterOpts,
    graph: GraphData,
    graphData: GraphData
) =>
{
    const diff = diffFn(graphData, graph);

    if (diff)
    {
        mergeFn(graph, diff, 'mutable');
    }

    return diff || null
};

const put = (
    opts: MemoryAdapterOpts,
    graph: GraphData,
    graphData: GraphData
): Promise<GraphData|null> => Promise.resolve(putSync(opts, graph, graphData));

export function createMemoryAdapter(
    opts: MemoryAdapterOpts = DEFAULT_OPTS
): GraphAdapter
{
    const graph: GraphData = {};

    return {
        get    : (soul: string) => get(opts, graph, soul),
        getSync: (soul: string) => getSync(opts, graph, soul),
        put    : (graphData: GraphData) => put(opts, graph, graphData),
        putSync: (graphData: GraphData) => putSync(opts, graph, graphData)
    }
}
