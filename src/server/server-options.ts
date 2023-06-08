import { TGSocketServerOptions } from 'topgun-socket/server';
import { TGGraphAdapter, TGGraphAdapterOptions } from '../types';

export interface TGServerOptions extends TGSocketServerOptions, TGGraphAdapterOptions
{
    disableValidation?: boolean;
    authMaxDrift?: number;
    ownerPub?: string;
    adapter?: TGGraphAdapter;
    port?: number;
}
