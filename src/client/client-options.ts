import { TGSocketClientOptions } from 'topgun-socket/client';
import { TGGraph } from './graph/graph';
import { TGGraphConnector } from './transports/graph-connector';
import { TGSupportedStorage } from '../types';
import { localStorageAdapter } from '../utils/local-storage';

export type TGClientPeerOptions = string|TGSocketClientOptions;

export interface TGClientOptions
{
    readonly peers?: TGClientPeerOptions[];
    readonly graph?: TGGraph;
    readonly connectors?: TGGraphConnector[];
    readonly persistStorage?: boolean;
    readonly storageKey?: string;
    readonly persistSession?: boolean;
    readonly sessionStorage?: TGSupportedStorage;
    readonly sessionStorageKey?: string;
    readonly passwordMinLength?: number;
}

export const DEFAULT_OPTIONS: Required<TGClientOptions> = {
    peers            : [],
    graph            : new TGGraph(),
    connectors       : [],
    persistStorage   : false,
    storageKey       : 'top-gun-nodes',
    persistSession   : true,
    sessionStorage   : localStorageAdapter,
    sessionStorageKey: 'top-gun-session',
    passwordMinLength: 8,
};
