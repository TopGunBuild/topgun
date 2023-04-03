import { TGServerOptions } from 'topgun-socket/server';
import { GraphAdapter } from '../types';

export interface SocketServerOptions extends TGServerOptions
{
    peers?: string[];
    disableValidation?: boolean;
    authMaxDrift?: number;
    ownerPub?: string;
    adapter?: GraphAdapter
}