import { cleanupTasks, randomPort, wait } from '@topgunbuild/test-utils';
import { ClientSocket, create, listen, SocketServer } from '..';

let server: SocketServer, client: ClientSocket;

describe('Server socket ping/pong test', () =>
{
    it('Should disconnect socket if server does not receive a pong from client before timeout', async () =>
    {
        const port = await randomPort();
        // Intentionally make pingInterval higher than pingTimeout, that
        // way the client will never receive a ping or send back a pong.
        server = listen(port, {
            pingInterval: 2000,
            pingTimeout : 500,
        });

        await server.listener('ready').once();

        client = create({
            hostname: '127.0.0.1',
            port,
        });

        let serverWarning = null;
        (async () =>
        {
            for await (let { warning } of server.listener('warning'))
            {
                serverWarning = warning;
            }
        })();

        let serverDisconnectionCode = null;
        (async () =>
        {
            for await (let event of server.listener('disconnection'))
            {
                serverDisconnectionCode = event.code;
            }
        })();

        let clientError = null;
        (async () =>
        {
            for await (let { error } of client.listener('error'))
            {
                clientError = error;
            }
        })();

        let clientDisconnectCode = null;
        (async () =>
        {
            for await (let event of client.listener('disconnect'))
            {
                clientDisconnectCode = event.code;
            }
        })();

        await wait(1000);
        expect(clientError).not.toEqual(null);
        expect(clientError.name).toEqual('SocketProtocolError');
        expect(clientDisconnectCode).toEqual(4000);

        expect(serverWarning).not.toEqual(null);
        expect(serverWarning.name).toEqual('SocketProtocolError');
        expect(serverDisconnectionCode).toEqual(4001);

        await new Promise<void>(resolve => setTimeout(() => resolve(), 2000));

        await cleanupTasks(client, server);
    });

    it('Should not disconnect socket if server does not receive a pong from client before timeout', async () =>
    {
        const port = await randomPort();

        // Intentionally make pingInterval higher than pingTimeout, that
        // way the client will never receive a ping or send back a pong.
        server = listen(port, {
            pingInterval       : 2000,
            pingTimeout        : 500,
            pingTimeoutDisabled: true,
        });

        await server.listener('ready').once();

        client = create({
            hostname           : '127.0.0.1',
            port               : port,
            pingTimeoutDisabled: true,
        });

        let serverWarning = null;
        (async () =>
        {
            for await (let { warning } of server.listener('warning'))
            {
                serverWarning = warning;
            }
        })();

        let serverDisconnectionCode = null;
        (async () =>
        {
            for await (let event of server.listener('disconnection'))
            {
                serverDisconnectionCode = event.code;
            }
        })();

        let clientError = null;
        (async () =>
        {
            for await (let { error } of client.listener('error'))
            {
                clientError = error;
            }
        })();

        let clientDisconnectCode = null;
        (async () =>
        {
            for await (let event of client.listener('disconnect'))
            {
                clientDisconnectCode = event.code;
            }
        })();

        await wait(1000);
        expect(clientError).toEqual(null);
        expect(clientDisconnectCode).toEqual(null);

        expect(serverWarning).toEqual(null);
        expect(serverDisconnectionCode).toEqual(null);

        await cleanupTasks(client, server);
    });
});

describe('Client socket ping/pong test', () =>
{
    it('Should disconnect if ping is not received before timeout', async () =>
    {
        const port = await randomPort();

        server = listen(port, {
            ackTimeout: 200,
        });
        await server.listener('ready').once();

        client = create({
            hostname      : '127.0.0.1',
            port,
            ackTimeout    : 200,
            connectTimeout: 500,
        });

        expect(client.pingTimeout).toEqual(500);

        (async () =>
        {
            for await (let _ of client.listener('connect'))
            {
                expect(client.transport.pingTimeout).toEqual(server.options.pingTimeout);
                // Hack to make the client ping independent from the server ping.
                client.transport.pingTimeout = 500;
                client.transport._resetPingTimeout();
            }
        })();

        let disconnectCode = null;
        let clientError    = null;

        (async () =>
        {
            for await (let { error } of client.listener('error'))
            {
                clientError = error;
            }
        })();

        (async () =>
        {
            for await (let event of client.listener('disconnect'))
            {
                disconnectCode = event.code;
            }
        })();

        await wait(1000);

        expect(disconnectCode).toEqual(4000);
        expect(clientError).not.toEqual(null);
        expect(clientError.name).toEqual('SocketProtocolError');

        await new Promise<void>(resolve => setTimeout(() => resolve(), 2000));
        await cleanupTasks(client, server);
    });

    it(
        'Should not disconnect if ping is not received before timeout when pingTimeoutDisabled is true',
        async () =>
        {
            const port = await randomPort();

            server = listen(port, {
                ackTimeout: 200,
            });
            await server.listener('ready').once();

            client = create({
                hostname           : '127.0.0.1',
                port,
                ackTimeout         : 200,
                connectTimeout     : 500,
                pingTimeoutDisabled: true,
            });

            expect(client.pingTimeout).toEqual(500);

            let clientError = null;
            (async () =>
            {
                for await (let { error } of client.listener('error'))
                {
                    clientError = error;
                }
            })();

            await wait(1000);
            expect(clientError).toEqual(null);
            await cleanupTasks(client, server);
        },
    );
});
