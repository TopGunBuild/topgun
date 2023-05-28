import { TGSocketServerOptions } from 'topgun-socket/server';
import { TGGraphAdapter } from '../types';

export interface TGServerOptions extends TGSocketServerOptions
{
    disableValidation?: boolean;
    authMaxDrift?: number;
    ownerPub?: string;
    adapter?: TGGraphAdapter;
    port?: number;
}
