import { LEX } from './lex';

export type ChangeSetEntry = readonly [string, TGGraphData];

/**
 * Timestamp of last change for each attribute
 */
export interface TGNodeState {
    [key: string]: number;
}

/**
 * Soul and State of a Node
 */
export interface TGNodeMeta {
    '#'?: string;
    '>'?: TGNodeState;
}

/**
 * A node (or partial node data) in a Graph
 */
export interface TGNode {
    _?: TGNodeMeta;

    [key: string]: any;
}

/**
 * Valid values in TopGunDB
 */
export type TGValue = object | string | number | boolean | null;

/**
 * Graph Data consists of one or more full or partial nodes
 */
export interface TGGraphData {
    [key: string]: TGNode | undefined;
}

export interface TGPartialGraphData {
    [key: string]: Partial<TGNode> | undefined;
}

export interface TGOptionsGet {
    /** soul */
    '#'?: string;
    /** LEX query */
    '.'?: LEX;
    /** Maximum number of key-value pairs to return */
    '%'?: number;
    /** true for reverse */
    '-'?: boolean;
}

export type TGOptionsPut = Partial<{
    opt: {
        /** certificate that gives other people write permission */ cert: string;
    };

    [key: string]: any;
}>;

export interface CRDTOpts {
    readonly machineState?: number;
    readonly futureGrace?: number;
    readonly Lexical?: (x: TGValue) => any;

    [k: string]: any;
}

export interface TGGraphAdapter {
    readonly close?: () => void;
    readonly get: (soul: string, opts?: TGOptionsGet) => Promise<TGNode | null>;
    readonly getJsonString?: (
        soul: string,
        opts?: TGOptionsGet,
    ) => Promise<string>;
    readonly getJsonStringSync?: (soul: string, opts?: TGOptionsGet) => string;
    readonly getSync?: (soul: string, opts?: TGOptionsGet) => TGNode | null;
    readonly put: (graphData: TGGraphData) => Promise<TGGraphData | null>;
    readonly putSync?: (graphData: TGGraphData) => TGGraphData | null;

    readonly pruneChangelog?: (before: number) => Promise<void>;

    readonly getChangesetFeed?: (
        from: string,
    ) => () => Promise<ChangeSetEntry | null>;

    readonly onChange?: (
        handler: (change: ChangeSetEntry) => void,
        from?: string,
    ) => () => void;
}
