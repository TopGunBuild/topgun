import { TGSocketClientOptions } from '@topgunbuild/socket/client';
import { TGGraphConnector } from './transports/graph-connector';
import { TGGraphAdapterOptions, TGSupportedStorage } from '../types';
import { localStorageAdapter } from '../utils/local-storage';
import { MAX_KEY_SIZE, MAX_VALUE_SIZE } from '../storage';

export type TGClientPeerOptions = string|TGSocketClientOptions;

export interface TGClientOptions extends TGGraphAdapterOptions
{
    peers?: TGClientPeerOptions[];
    connectors?: TGGraphConnector[];
    localStorage?: boolean;
    localStorageKey?: string;
    sessionStorage?: TGSupportedStorage|boolean;
    sessionStorageKey?: string;
    passwordMinLength?: number;
    passwordMaxLength?: number;
    transportMaxKeyValuePairs?: number;
}

export const TG_CLIENT_DEFAULT_OPTIONS: Required<TGClientOptions> = {
    peers                    : [],
    connectors               : [],
    localStorage             : false,
    localStorageKey          : 'topgun-nodes',
    sessionStorage           : localStorageAdapter,
    sessionStorageKey        : 'topgun-session',
    passwordMinLength        : 8,
    passwordMaxLength        : 48,
    transportMaxKeyValuePairs: 200,
    maxKeySize               : MAX_KEY_SIZE,
    maxValueSize             : MAX_VALUE_SIZE
};
