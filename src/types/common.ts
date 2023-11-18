import { TGSocketClientOptions } from '@topgunbuild/socket/client';

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
 * Valid values in TopGunDB
 */
export type TGValue = object|string|number|boolean|null;

export interface TGPartialNode
{
    [key: string]: TGValue;
}

export interface TGNode extends TGPartialNode
{
    _: TGNodeMeta;
}

export interface TGRefNode extends TGNode
{
    '#': string;
}

/**
 * Graph Data consists of one or more full or partial nodes
 */
export interface TGGraphData
{
    [key: string]: TGNode|null;
}

export interface TGPartialGraphData
{
    [key: string]: TGPartialNode|null;
}

export type Lex = {
    /** prefix match */
    '*'?: string;
    /** greater than or equals */
    '>'?: string;
    /** less than match */
    '<'?: string;
};

export interface TGOptionsGet extends Lex
{
    /** soul */
    '#'?: string;
    /** true for reverse */
    '-'?: boolean;
    /** Maximum number of key-value pairs to return */
    '%'?: number;
}

export interface IPolicyLex extends Lex {
    /** Path */
    '#'?: IPolicyLex;
    /** Key */
    '.'?: IPolicyLex;
    /** equals this string */
    '='?: string;
    /**
     * Either Path string or Key string must
     * contain Certificate's Pub string
     */
    '+'?: '*';
}

export type IPolicy = string | IPolicyLex | (string | IPolicyLex)[];

export type TGOptionsPut = Partial<{
    opt: {
        /** certificate that gives other people write permission */ cert: string;
    };

    [key: string]: any;
}>;


export interface TGCollectionOptions
{
    idField?: string;
    keyField?: string;
}

export interface CRDTOptions
{
    machineState?: number;
    futureGrace?: number;
    Lexical?: (x: TGValue) => any;

    [k: string]: any;
}

export type TGOriginators = Record<string, number>;

/**
 * A standard Protocol Message
 */
export interface TGMessage
{
    '#'?: string;
    '@'?: string;

    get?: TGOptionsGet;
    put?: {
        [key: string]: TGPartialNode|null
    };

    ack?: number|boolean;
    err?: any;
    ok?: boolean|number;
    originators?: TGOriginators;
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
    originators?: TGOriginators;
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
    once?: boolean;
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

export interface TGEncryptData
{
    ct: string;
    iv: string;
    s: string;
    readonly e?: number;
    readonly w?: Record<string, string>[];
    readonly c?: string;
    readonly wb?: string;
}

export interface TGUserGraph
{
    alias: string;
    auth: {
        ek: TGEncryptData,
        s: string;
    },
    epub: string;
    pub: string;

    [key: string]: TGValue;
}

export type TGAuthCallback = (userRef?: TGUserCredentials|TGAckErr) => void;

export type TGOnCb<T> = (node: T, soul?: string) => void;

export interface TGData<T>
{
    value: T;
    key: string;
}

export interface TGCollectionChangeEvent
{
    oldValue: TGNode[];
    newValue: TGNode[];
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
    opts?: CRDTOptions|TGOptionsGet|TGOptionsPut,
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

export type TGPeerOptions = string|TGSocketClientOptions;

export interface TGGraphAdapter
{
    readonly close?: () => void;
    readonly get: (opts: TGOptionsGet) => Promise<TGGraphData>;
    readonly put: (graphData: TGPartialGraphData, originators?: TGOriginators) => Promise<TGGraphData|null>;
    readonly onChange?: (
        handler: (change: TGGraphData) => void
    ) => () => void
}

export interface TGGraphAdapterOptions
{
    maxKeySize?: number;
    maxValueSize?: number;
}

export interface TGPolicyLexUIOptions
{
    start?: string;
    end?: string;
    equals?: string;
    prefix?: string;
}

export interface TGPolicyOptions extends TGPolicyLexUIOptions
{
    pubInPatch?: boolean;
    key?: TGPolicyLexUIOptions;
    path?: TGPolicyLexUIOptions;
}
