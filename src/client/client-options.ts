import { TGGraph } from './graph/graph';
import { TGGraphConnector } from './transports/graph-connector';
import { TGSupportedStorage } from '../types';

export interface TGClientOptions {
    readonly peers?: string[];
    readonly graph?: TGGraph;
    readonly connectors?: TGGraphConnector[];
    readonly persistStorage?: boolean;
    readonly storageKey?: string;
    readonly persistSession?: boolean;
    readonly sessionStorage?: TGSupportedStorage;
    readonly sessionStorageKey?: string;
    readonly passwordMinLength?: number;
}
