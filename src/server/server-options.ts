import { TGSocketServerOptions } from '@topgunbuild/socket/server';
import { TGGraphAdapter, TGGraphAdapterOptions, TGPeerOptions } from '../types';

export interface TGServerOptions extends TGSocketServerOptions, TGGraphAdapterOptions
{
    disableValidation?: boolean;
    authMaxDrift?: number;
    ownerPub?: string;
    adapter?: TGGraphAdapter;
    port?: number;
    peers?: TGPeerOptions[];
}
