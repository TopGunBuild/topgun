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

describe('Errors', () =>
{
    it(
        'Should be able to emit the error event locally on the socket',
        async () =>
        {
            let err = null;

            (async () =>
            {
                for await (let { error } of client.listener('error'))
                {
                    err = error;
                }
            })();

            (async () =>
            {
                for await (let _ of client.listener('connect'))
                {
                    let error  = new Error('Custom error');
                    error.name = 'CustomError';
                    client.emit('error', { error });
                }
            })();

            await wait(100);

            expect(err).not.toEqual(null);
            expect(err.name).toEqual('CustomError');
        },
    );
});
