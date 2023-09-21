import { TGSocketServerOptions } from '@topgunbuild/socket/server';
import { TGGraphAdapter, TGGraphAdapterOptions, TGPeerOptions } from '../types';
import { TGLoggerOptions } from '../logger';

export interface TGServerOptions extends TGSocketServerOptions, TGGraphAdapterOptions
{
    disableValidation?: boolean;
    authMaxDrift?: number;
    ownerPub?: string;
    adapter?: TGGraphAdapter;
    port?: number;
    peers?: TGPeerOptions[];
    peerSyncInterval?: number;
    peerPruneInterval?: number;
    peerBackSync?: number;
    peerMaxStaleness?: number;
    peerBatchInterval?: number;
    peerChangelogRetention?: number;
    log?: TGLoggerOptions;
}
