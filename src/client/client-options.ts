import { TGGraphConnector } from './transports/graph-connector';
import { TGPeerOptions, TGGraphAdapterOptions, TGSupportedStorage } from '../types';

export interface TGClientOptions extends TGGraphAdapterOptions
{
    peers?: TGPeerOptions[];
    connectors?: TGGraphConnector[];
    localStorage?: boolean;
    localStorageKey?: string;
    sessionStorage?: TGSupportedStorage|boolean;
    sessionStorageKey?: string;
    passwordMinLength?: number;
    passwordMaxLength?: number;
    transportMaxKeyValuePairs?: number;
}
