import { ChangeSetEntry, TGGraphData, TGNode, TGOptionsGet } from './common';

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
