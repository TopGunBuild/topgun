import { CRDTOpts, GraphData, OptionsGet, OptionsPut, Value } from './graph-adapter';

/**
 * A standard Protocol Message
 */
export interface Message
{
    '#'?: string
    '@'?: string

    readonly get?: {
        readonly '#': string
    }

    readonly put?: GraphData

    readonly ack?: number|boolean
    readonly err?: any
    readonly ok?: boolean|number
}

export type MessageCb = (msg: Message) => void

/**
 * How puts are communicated to connectors
 */
export interface Put
{
    readonly graph: GraphData
    readonly msgId?: string
    readonly replyTo?: string
    readonly cb?: MessageCb
}

/**
 * How gets are communicated to connectors
 */
export interface Get
{
    readonly soul: string;
    readonly opts?: OptionsGet,
    readonly msgId?: string;
    readonly key?: string;
    readonly cb?: MessageCb;
}

export interface UserReference
{
    readonly alias: string
    readonly pub: string
}

export interface AckErr
{
    readonly err: Error
}

export interface UserCredentials
{
    readonly priv: string
    readonly epriv: any
    readonly alias: string
    readonly pub: string
    readonly epub: string
}

export type AuthCallback = (userRef?: UserReference|AckErr) => void;

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
type MaybePromisify<T> = T|Promise<T>

type PromisifyMethods<T> = {
    [K in keyof T]: T[K] extends AnyFunction
        ? (...args: Parameters<T[K]>) => MaybePromisify<ReturnType<T[K]>>
        : T[K]
}

export type SupportedStorage = PromisifyMethods<Pick<Storage, 'getItem'|'setItem'|'removeItem'>>


