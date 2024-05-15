import { cleanupTasks, randomPort, wait } from '@topgunbuild/test-utils';
import { AsyncFunction } from '@topgunbuild/utils';
import { ClientSocket, create, listen, SocketServer } from '../src';

let server: SocketServer,
    client: ClientSocket,
    middlewareFunction: AsyncFunction;

beforeEach(async () =>
{
    const port = await randomPort();
    server     = listen(port);
    client     = create({
        hostname: '127.0.0.1',
        port,
    });
});

afterEach(async () =>
{
    await cleanupTasks(client, server);
});

describe('Middleware', () =>
{
    describe('MIDDLEWARE_HANDSHAKE_TG', () =>
    {
        it('Should trigger correct events if MIDDLEWARE_HANDSHAKE_TG blocks with an error', async () =>
        {
            let middlewareWasExecuted = false;
            let serverWarnings        = [];
            let clientErrors          = [];
            let abortStatus;

            middlewareFunction = async function()
            {
                await wait(100);
                middlewareWasExecuted = true;
                let err               = new Error('Handshake failed because the server was too lazy');
                err.name              = 'TooLazyHandshakeError';
                throw err;
            };
            server.addMiddleware(server.MIDDLEWARE_HANDSHAKE_TG, middlewareFunction);

            (async () =>
            {
                for await (let { warning } of server.listener('warning'))
                {
                    serverWarnings.push(warning);
                }
            })();

            (async () =>
            {
                for await (let { error } of client.listener('error'))
                {
                    clientErrors.push(error);
                }
            })();

            (async () =>
            {
                let event   = await client.listener('connectAbort').once();
                abortStatus = event.code;
            })();

            await wait(200);
            expect(middlewareWasExecuted).toEqual(true);
            expect(clientErrors[0]).not.toEqual(null);
            expect(clientErrors[0].name).toEqual('TooLazyHandshakeError');
            expect(clientErrors[1]).not.toEqual(null);
            expect(clientErrors[1].name).toEqual('SocketProtocolError');
            expect(serverWarnings[0]).not.toEqual(null);
            expect(serverWarnings[0].name).toEqual('TooLazyHandshakeError');
            expect(abortStatus).not.toEqual(null);
        });

        it('Should send back default 4008 status code if MIDDLEWARE_HANDSHAKE_TG blocks without providing a status code', async () =>
        {
            let middlewareWasExecuted = false;
            let abortStatus: number;
            let abortReason: string;

            middlewareFunction = async function()
            {
                await wait(100);
                middlewareWasExecuted = true;
                let err               = new Error('Handshake failed because the server was too lazy');
                err.name              = 'TooLazyHandshakeError';
                throw err;
            };
            server.addMiddleware(server.MIDDLEWARE_HANDSHAKE_TG, middlewareFunction);

            (async () =>
            {
                let event   = await client.listener('connectAbort').once();
                abortStatus = event.code;
                abortReason = event.reason;
            })();

            await wait(200);
            expect(middlewareWasExecuted).toEqual(true);
            expect(abortStatus).toEqual(4008);
            expect(abortReason).toEqual(
                'TooLazyHandshakeError: Handshake failed because the server was too lazy',
            );
        });

        it('Should send back custom status code if MIDDLEWARE_HANDSHAKE_TG blocks by providing a status code', async () =>
        {
            let middlewareWasExecuted = false;
            let abortStatus: number;
            let abortReason: string;

            middlewareFunction = async function()
            {
                await wait(100);
                middlewareWasExecuted = true;
                let err               = new Error('Handshake failed because of invalid query auth parameters');
                err.name              = 'InvalidAuthQueryHandshakeError';
                // Set custom 4501 status code as a property of the error.
                // We will treat this code as a fatal authentication failure on the front end.
                // A status code of 4500 or higher means that the client shouldn't try to reconnect.
                err['statusCode'] = 4501;
                throw err;
            };
            server.addMiddleware(server.MIDDLEWARE_HANDSHAKE_TG, middlewareFunction);

            (async () =>
            {
                let event   = await client.listener('connectAbort').once();
                abortStatus = event.code;
                abortReason = event.reason;
            })();

            await wait(200);
            expect(middlewareWasExecuted).toEqual(true);
            expect(abortStatus).toEqual(4501);
            expect(abortReason).toEqual(
                'InvalidAuthQueryHandshakeError: Handshake failed because of invalid query auth parameters',
            );
        });

        it('Should connect with a delay if next() is called after a timeout inside the middleware function', async () =>
        {
            let createConnectionTime = null;
            let connectEventTime     = null;

            middlewareFunction = async () => await wait(500);
            server.addMiddleware(server.MIDDLEWARE_HANDSHAKE_TG, middlewareFunction);

            createConnectionTime = Date.now();

            await client.listener('connect').once();
            connectEventTime = Date.now();
            expect(connectEventTime - createConnectionTime > 400).toEqual(true);
        });
    });
});
