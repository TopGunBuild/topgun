import { cleanupTasks, randomPort, wait } from '@topgunbuild/test-utils';
import { ClientSocket, create, listen, SocketServer } from '..';

let server: SocketServer, client: ClientSocket;

beforeEach(async () =>
{
    const port = await randomPort();
    server     = listen(port, {
        ackTimeout: 200,
    });
    client     = create({
        hostname  : '127.0.0.1',
        port,
        ackTimeout: 200,
    });

    await server.listener('ready').once();
});

afterEach(async () =>
{
    await cleanupTasks(client, server);
});

describe('Emitting remote events', () =>
{
    it(
        'Should not throw error on socket if ackTimeout elapses before response to event is sent back',
        async () =>
        {
            let caughtError: Error, responseError: Error;

            for await (let _ of client.listener('connect'))
            {
                try
                {
                    await client.invoke('performTask', 123);
                }
                catch (err)
                {
                    responseError = err as Error;
                }
                await wait(250);
                try
                {
                    client.disconnect();
                }
                catch (err)
                {
                    caughtError = err as Error;
                }
                break;
            }

            expect(responseError).not.toEqual(null);
            expect(caughtError).toBeUndefined();
        },
    );
});
