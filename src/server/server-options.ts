import { TGSocketServerOptions } from '@topgunbuild/socket/server';
import { TGGraphAdapter, TGGraphAdapterOptions, TGPeerOptions } from '../types';
import { TGLoggerOptions } from '../logger';

export interface TGServerOptions extends TGSocketServerOptions, TGGraphAdapterOptions
{
    disableGraphValidation?: boolean;
    adapter?: TGGraphAdapter;
    port?: number;
    log?: TGLoggerOptions;
    serverName?: string;
    peers?: TGPeerOptions[];
    putToPeers?: boolean;
    reversePeerSync?: boolean;
    peerSecretKey?: string;
}
