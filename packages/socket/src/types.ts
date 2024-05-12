import { Socket } from './server/socket';

export interface EventObject
{
    id?: any;
    useCache?: boolean;
    channel?: string;
    pingTimeout?: number;
    socket?: Socket;
    event?: string;
    data?: any;
    callback?: EventObjectCallback|undefined;
    cid?: number|undefined;
    timeout?: any;
    rid?: any;
    error?: any;
}

export type EventObjectCallback = (
    error: Error,
    eventObject: EventObject,
) => void;

export type SocketState = 'connecting'|'open'|'closed';

export interface CodecEngine {
    decode: (input: any) => any;
    encode: (object: any) => any;
}

export const MIDDLEWARE_HANDSHAKE_WS = 'handshakeWS';
export const MIDDLEWARE_HANDSHAKE_AG = 'handshakeAG';
export const MIDDLEWARE_TRANSMIT     = 'transmit';
export const MIDDLEWARE_INVOKE       = 'invoke';
export const MIDDLEWARE_SUBSCRIBE    = 'subscribe';
export const MIDDLEWARE_PUBLISH_IN   = 'publishIn';
export const MIDDLEWARE_PUBLISH_OUT  = 'publishOut';

export type Middlewares =
    typeof MIDDLEWARE_HANDSHAKE_WS
    |typeof MIDDLEWARE_HANDSHAKE_AG
    |typeof MIDDLEWARE_TRANSMIT
    |typeof MIDDLEWARE_INVOKE
    |typeof MIDDLEWARE_SUBSCRIBE
    |typeof MIDDLEWARE_PUBLISH_IN
    |typeof MIDDLEWARE_PUBLISH_OUT;

export type MiddlewareFunction = (request: any) => Promise<void>;

