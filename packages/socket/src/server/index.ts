import { Server } from 'http';
import { isNode } from '@topgunbuild/utils';
import { SocketServer } from './server';
import { SocketServerOptions } from './types';

export * from './server';
export * from './socket';
export * from './types';

type ListenFn = (...rest: any[]) => void;

/**
 * Captures upgrade requests for a http.Server.
 */
export function attach(server: Server, options?: SocketServerOptions): SocketServer
{
    options            = options || {};
    options.httpServer = server;
    return new SocketServer(options);
}

export function listen(
    port: number,
    options?: SocketServerOptions|ListenFn,
    fn?: ListenFn,
): SocketServer
{
    if (typeof options === 'function')
    {
        fn      = options;
        options = {};
    }

    if (isNode())
    {
        const http       = require('http');
        const httpServer = http.createServer();

        const gateway      = attach(httpServer, options);
        gateway.httpServer = httpServer;
        gateway.httpServer.listen(port, fn);

        return gateway;
    }
    else
    {
        return new SocketServer(options);
    }
}
