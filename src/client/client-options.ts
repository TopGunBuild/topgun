import { TGSocketClientOptions } from 'topgun-socket/client';
import { TGGraph } from './graph/graph';
import { TGGraphConnector } from './transports/graph-connector';
import { TGSupportedStorage } from '../types';
import { localStorageAdapter } from '../utils/local-storage';

export type TGClientPeerOptions = string|TGSocketClientOptions;

export interface TGClientOptions
{
    peers?: TGClientPeerOptions[];
    graph?: TGGraph;
    connectors?: TGGraphConnector[];
    persistStorage?: boolean;
    storageKey?: string;
    persistSession?: boolean;
    sessionStorage?: TGSupportedStorage;
    sessionStorageKey?: string;
    passwordMinLength?: number;
    passwordMaxLength?: number;
    transportMaxKeyValuePairs?: number;
}

export const DEFAULT_OPTIONS: Required<TGClientOptions> = {
    peers                    : [],
    graph                    : new TGGraph(),
    connectors               : [],
    persistStorage           : false,
    storageKey               : 'topgun-nodes',
    persistSession           : true,
    sessionStorage           : localStorageAdapter,
    sessionStorageKey        : 'topgun-session',
    passwordMinLength        : 8,
    passwordMaxLength        : 48,
    transportMaxKeyValuePairs: 200
};
