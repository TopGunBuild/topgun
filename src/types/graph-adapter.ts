import { TGGraphData, TGOptionsGet } from './common';

export interface TGGraphAdapter
{
    readonly close?: () => void;
    readonly get: (opts: TGOptionsGet) => Promise<TGGraphData>;
    readonly put: (graphData: TGGraphData) => Promise<TGGraphData|null>;
}

export interface TGGraphAdapterOptions
{
    maxKeySize?: number;
    maxValueSize?: number;
}