import { SimpleBroker } from '../simple-broker';
import { Socket } from './socket';
import { CodecEngine } from '../types';

export interface SocketServerOptions {
    appName?: string;

    // An instance of a Node.js HTTP server.
    // https://nodejs.org/api/http.html#http_class_http_server
    // This option should not be set if the server is created
    // with socketClusterServer.attach(...).
    httpServer?: any;

    // This can be the name of an npm module or a path to a
    // Node.js module to use as the WebSocket server engine.
    wsEngine?: string | { Server: any };

    // Custom options to pass to the wsEngine when it is being
    // instantiated.
    wsEngineServerOptions?: any;

    brokerEngine?: SimpleBroker;

    pubSubBatchDuration?: number;

    // Can be 1 or 2. Version 1 is for maximum backwards
    // compatibility with TopGunSocket clients.
    protocolVersion?: 1 | 2;

    // In milliseconds - If the socket handshake hasn't been
    // completed before this timeout is reached, the new
    // connection attempt will be terminated.
    handshakeTimeout?: number;

    // In milliseconds, the timeout for receiving a response
    // when using invoke() or invokePublish().
    ackTimeout?: number;

    // Origins which are allowed to connect to the server.
    origins?: string;

    // The maximum number of unique channels which a single
    // socket can subscribe to.
    socketChannelLimit?: number;

    // The interval in milliseconds on which to
    // send a ping to the client to check that
    // it is still alive.
    pingInterval?: number;

    // How many milliseconds to wait without receiving a ping
    // before closing the socket.
    pingTimeout?: number;

    pingTimeoutDisabled?: boolean;

    // Whether or not an error should be emitted on
    // the socket whenever an action is blocked by a
    // middleware function
    middlewareEmitFailures?: boolean;

    // The URL path reserved by TopGunSocket clients to
    // interact with the server.
    path?: string;

    // Whether or not clients are allowed to publish messages
    // to channels.
    allowClientPublish?: boolean;

    // Whether or not to batch all socket messages
    // for some time immediately after completing
    // a handshake. This can be useful in failure-recovery
    // scenarios (e.g. batch resubscribe).
    batchOnHandshake?: boolean;

    // If batchOnHandshake is true, this lets you specify
    // How long to enable batching (in milliseconds) following
    // a successful socket handshake.
    batchOnHandshakeDuration?: number;

    // If batchOnHandshake is true, this lets you specify
    // the size of each batch in milliseconds.
    batchInterval?: number;

    // Lets you specify the default cleanup behaviour for
    // when a socket becomes disconnected.
    // Can be either 'kill' or 'close'. Kill mode means
    // that all of the socket's streams will be killed and
    // so consumption will stop immediately.
    // Close mode means that consumers on the socket will
    // be able to finish processing their stream backlogs
    // bebfore they are ended.
    socketStreamCleanupMode?: 'kill' | 'close';

    codecEngine?: CodecEngine;
    middlewareEmitWarnings?: boolean;
}

export interface IncomingMessage {
    remoteAddress?: string;
    remoteFamily?: any;
    remotePort?: number;
    forwardedForAddress?: any;
}

export interface RequestObject
{
    socket?: Socket;
    channel?: string;
    data?: any;
    ackData?: any;
    event?: string;
}

export interface ServerChannelOptions
{
    channel: string;
    data?: any;
}
