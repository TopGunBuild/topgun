import {
    CRDTOpts,
    TGGraphData,
    TGNode,
    TGOptionsGet,
    TGOptionsPut,
    TGValue,
} from './graph-adapter';

/**
 * A standard Protocol Message
 */
export interface TGMessage {
    '#'?: string;
    '@'?: string;

    readonly get?: {
        readonly '#': string;
    };

    readonly put?: TGGraphData;

    readonly ack?: number | boolean;
    readonly err?: any;
    readonly ok?: boolean | number;
}

export type TGMessageCb = (msg: TGMessage) => void;

/**
 * How puts are communicated to connectors
 */
export interface TGPut {
    readonly graph: TGGraphData;
    readonly msgId?: string;
    readonly replyTo?: string;
    readonly cb?: TGMessageCb;
}

/**
 * How gets are communicated to connectors
 */
export interface TGGet {
    readonly soul: string;
    readonly opts?: TGOptionsGet;
    readonly msgId?: string;
    readonly key?: string;
    readonly cb?: TGMessageCb;
}

export interface TGUserReference {
    readonly alias: string;
    readonly pub: string;
}

export interface TGAckErr {
    readonly err: Error;
}

export interface TGUserCredentials {
    readonly priv: string;
    readonly epriv: any;
    readonly alias: string;
    readonly pub: string;
    readonly epub: string;
}

export type TGAuthCallback = (userRef?: TGUserReference | TGAckErr) => void;

export interface TGChainOptions {
    readonly uuid?: (path: readonly string[]) => Promise<string> | string;
}

export type TGOnCb = (node: TGValue | undefined, key?: string) => void;
export type TGNodeListenCb = (node: TGNode | undefined) => void;

export interface TGPathData {
    readonly souls: readonly string[];
    readonly value: TGValue | undefined;
    readonly complete: boolean;
}

export type TGMiddleware = (
    updates: TGGraphData,
    existingGraph: TGGraphData,
    opts?: CRDTOpts | TGOptionsGet | TGOptionsPut,
    fullPath?: string[],
) => TGGraphData | undefined | Promise<TGGraphData | undefined>;
export type TGMiddlewareType = 'read' | 'write';

type AnyFunction = (...args: any[]) => any;
type MaybePromisify<T> = T | Promise<T>;

type PromisifyMethods<T> = {
    [K in keyof T]: T[K] extends AnyFunction
        ? (...args: Parameters<T[K]>) => MaybePromisify<ReturnType<T[K]>>
        : T[K];
};

export type TGSupportedStorage = PromisifyMethods<
    Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>
>;
