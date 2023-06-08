import { TGGraphData, TGOptionsGet } from './common';

export interface TGGraphAdapter
{
    readonly close?: () => void;
    readonly get: (soul: string, opts?: TGOptionsGet) => Promise<TGGraphData>;
    readonly put: (graphData: TGGraphData) => Promise<TGGraphData|null>;
}

export interface TGGraphAdapterOptions
{
    maxKeySize?: number;
    maxValueSize?: number;
}