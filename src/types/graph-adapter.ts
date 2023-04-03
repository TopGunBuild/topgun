import { LEX } from './lex';

export type ChangeSetEntry = readonly [string, GraphData];

/**
 * Timestamp of last change for each attribute
 */
export interface NodeState
{
    [key: string]: number
}

/**
 * Soul and State of a Node
 */
export interface NodeMeta
{
    '#'?: string;
    '>'?: NodeState;
}

/**
 * A node (or partial node data) in a Graph
 */
export interface Node
{
    _?: NodeMeta
    [key: string]: any
}

/**
 * Valid values in TopGunDB
 */
export type Value = object|string|number|boolean|null;

/**
 * Graph Data consists of one or more full or partial nodes
 */
export interface GraphData
{
    [key: string]: Node|undefined
}

export interface PartialGraphData
{
    [key: string]: Partial<Node>|undefined
}

export interface OptionsGet
{
    /** soul */
    '#'?: string;
    /** LEX query */
    '.'?: LEX;
    /** Maximum number of key-value pairs to return */
    '%'?: number;
    /** true for reverse */
    '-'?: boolean;
}

export type OptionsPut = Partial<{
    opt: {
        /** certificate that gives other people write permission */ cert: string;
    };
}>;

export interface CRDTOpts
{
    readonly machineState?: number
    readonly futureGrace?: number
    readonly Lexical?: (x: Value) => any
}

export interface GraphAdapter
{
    readonly close?: () => void
    readonly get: (soul: string, opts?: OptionsGet) => Promise<Node|null>
    readonly getJsonString?: (soul: string, opts?: OptionsGet) => Promise<string>
    readonly getJsonStringSync?: (soul: string, opts?: OptionsGet) => string
    readonly getSync?: (soul: string, opts?: OptionsGet) => Node|null
    readonly put: (graphData: GraphData) => Promise<GraphData|null>
    readonly putSync?: (graphData: GraphData) => GraphData|null

    readonly pruneChangelog?: (before: number) => Promise<void>

    readonly getChangesetFeed?: (
        from: string
    ) => () => Promise<ChangeSetEntry|null>

    readonly onChange?: (
        handler: (change: ChangeSetEntry) => void,
        from?: string
    ) => () => void
}
