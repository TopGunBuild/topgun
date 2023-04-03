import { CRDTOpts, OptionsGet, GraphData, Node, OptionsPut, Value } from '../types'

export interface ChainOptions
{
    readonly uuid?: (path: readonly string[]) => Promise<string>|string
}

export type OnCb = (node: Value|undefined, key?: string) => void;
export type NodeListenCb = (node: Node|undefined) => void

export interface PathData
{
    readonly souls: readonly string[]
    readonly value: Value|undefined
    readonly complete: boolean
}

export type Middleware = (
    updates: GraphData,
    existingGraph: GraphData,
    opts?: CRDTOpts|OptionsGet|OptionsPut,
    fullPath?: string[]
) => GraphData|undefined|Promise<GraphData|undefined>;
export type MiddlewareType = 'read'|'write';

type AnyFunction = (...args: any[]) => any
type MaybePromisify<T> = T | Promise<T>

type PromisifyMethods<T> = {
    [K in keyof T]: T[K] extends AnyFunction
        ? (...args: Parameters<T[K]>) => MaybePromisify<ReturnType<T[K]>>
        : T[K]
}

export type SupportedStorage = PromisifyMethods<Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>>
