import { LEX } from './lex';

/**
 * Timestamp of last change for each attribute
 */
export interface TGNodeState
{
    [key: string]: number;
}

/**
 * Soul and State of a Node
 */
export interface TGNodeMeta
{
    '#': string;
    '>': TGNodeState;
}

/**
 * A node (or partial node data) in a Graph
 */
export interface TGNode
{
    _: TGNodeMeta;

    [key: string]: any;
}

/**
 * Valid values in TopGunDB
 */
export type TGValue = object|string|number|boolean|null;

/**
 * Graph Data consists of one or more full or partial nodes
 */
export interface TGGraphData
{
    [key: string]: TGNode|null;
}

export interface TGOptionsGet
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

export type TGOptionsPut = Partial<{
    opt: {
        /** certificate that gives other people write permission */ cert: string;
    };

    [key: string]: any;
}>;

export interface CRDTOpts
{
    machineState?: number;
    futureGrace?: number;
    Lexical?: (x: TGValue) => any;

    [k: string]: any;
}

/**
 * A standard Protocol Message
 */
export interface TGMessage
{
    '#'?: string;
    '@'?: string;

    get?: TGOptionsGet;
    put?: TGGraphData;

    ack?: number|boolean;
    err?: any;
    ok?: boolean|number;
}

export type TGMessageCb = (msg: TGMessage) => void;

/**
 * How puts are communicated to connectors
 */
export interface TGPut
{
    graph: TGGraphData;
    msgId?: string;
    replyTo?: string;
    cb?: TGMessageCb;
}

/**
 * How gets are communicated to connectors
 */
export interface TGGet
{
    options: TGOptionsGet;
    msgId?: string;
    key?: string;
    cb?: TGMessageCb;
}

export interface TGUserReference
{
    alias: string;
    pub: string;
}

export interface TGAckErr
{
    err: Error;
}

export interface TGUserCredentials
{
    priv: string;
    epriv: any;
    alias: string;
    pub: string;
    epub: string;
}

export type TGAuthCallback = (userRef?: TGUserReference|TGAckErr) => void;

export type TGOnCb<T extends TGValue> = (node: T, soul?: string) => void;

export interface TGData<T extends TGValue>
{
    value: T;
    soul: string;
}

export interface TGPathData
{
    souls: string[];
    value: TGValue|undefined;
    complete: boolean;
}

export type TGMiddleware = (
    updates: TGGraphData,
    existingGraph: TGGraphData,
    opts?: CRDTOpts|TGOptionsGet|TGOptionsPut,
    fullPath?: string[],
) => TGGraphData|undefined|Promise<TGGraphData|undefined>;
export type TGMiddlewareType = 'read'|'write';

type AnyFunction = (...args: any[]) => any;
type MaybePromisify<T> = T|Promise<T>;

type PromisifyMethods<T> = {
    [K in keyof T]: T[K] extends AnyFunction
        ? (...args: Parameters<T[K]>) => MaybePromisify<ReturnType<T[K]>>
        : T[K];
};

export type TGSupportedStorage = PromisifyMethods<Pick<Storage, 'getItem'|'setItem'|'removeItem'>>;
