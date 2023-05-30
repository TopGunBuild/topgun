import { TGGraphData, TGNode, TGOptionsGet } from './common';

export interface TGGraphAdapter
{
    readonly close?: () => void;
    readonly get: (soul: string, opts?: TGOptionsGet) => Promise<TGNode|null>;
    readonly put: (graphData: TGGraphData) => Promise<TGGraphData|null>;
}
