import { ClientSocket, create, listen, SocketServer } from '..';
import { cleanupTasks, randomPort } from '@topgunbuild/test-utils';

let client: ClientSocket, server: SocketServer;

beforeEach(async () =>
{
    const port = await randomPort();

    server = listen(port, {
        ackTimeout: 200
    });
    client     = create({
        hostname  : '127.0.0.1',
        port,
        ackTimeout: 200
    });

    await server.listener('ready').once();
});

afterEach(async () =>
{
    await cleanupTasks(client, server);
});

describe('Reconnecting socket', () =>
{
    it('Should disconnect socket with code 1000 and reconnect', async () =>
    {
        await client.listener('connect').once();

        let disconnectCode: number;
        let disconnectReason: string|undefined;

        (async () =>
        {
            for await (let event of client.listener('disconnect'))
            {
                disconnectCode   = event.code;
                disconnectReason = event.reason;
            }
        })();

        client.reconnect();
        await client.listener('connect').once();

        expect(disconnectCode).toEqual(1000);
        expect(disconnectReason).toEqual(undefined);
    });

    it(
        'Should disconnect socket with custom code and data when socket.reconnect() is called with arguments',
        async () =>
        {
            await client.listener('connect').once();

            let disconnectCode: number;
            let disconnectReason: string|undefined;

            (async () =>
            {
                let event        = await client.listener('disconnect').once();
                disconnectCode   = event.code;
                disconnectReason = event.reason;
            })();

            client.reconnect(1000, 'About to reconnect');
            await client.listener('connect').once();

            expect(disconnectCode).toEqual(1000);
            expect(disconnectReason).toEqual('About to reconnect');
        }
    );
});
