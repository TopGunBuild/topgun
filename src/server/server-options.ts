import { TGServerSocketGatewayOptions } from 'topgun-socket/server';
import { TGGraphAdapter } from '../types';

export interface TGServerOptions extends TGServerSocketGatewayOptions
{
    disableValidation?: boolean;
    authMaxDrift?: number;
    ownerPub?: string;
    adapter?: TGGraphAdapter;
    port?: number;
}
