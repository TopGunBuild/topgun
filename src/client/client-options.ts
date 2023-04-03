import { Graph } from './graph/graph';
import { GraphConnector } from './transports/graph-connector';
import { SupportedStorage } from './interfaces';

export interface ClientOptions
{
    readonly peers?: string[];
    readonly graph?: Graph;
    readonly connectors?: GraphConnector[];
    readonly persistStorage?: boolean;
    readonly storageKey?: string;
    readonly persistSession?: boolean;
    readonly sessionStorage?: SupportedStorage;
    readonly sessionStorageKey?: string;
    readonly passwordMinLength?: number;
}
