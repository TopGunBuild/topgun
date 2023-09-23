import { TGSocketServerOptions } from '@topgunbuild/socket/server';
import { TGGraphAdapter, TGGraphAdapterOptions, TGPeerOptions } from '../types';
import { TGLoggerOptions } from '../logger';

export interface TGServerOptions extends TGSocketServerOptions, TGGraphAdapterOptions
{
    disableValidation?: boolean;
    ownerPub?: string;
    adapter?: TGGraphAdapter;
    port?: number;
    peers?: TGPeerOptions[];
    putToPeers?: boolean;
    reversePeerSync?: boolean;
    log?: TGLoggerOptions;
}
