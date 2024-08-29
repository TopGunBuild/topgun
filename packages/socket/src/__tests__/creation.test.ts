import { cleanupTasks, randomPort } from '@topgunbuild/test-utils';
import { ClientSocket, create, SocketClientOptions } from '..';

let client: ClientSocket, clientOptions: SocketClientOptions;

afterEach(async () =>
{
    await cleanupTasks(client);
});

describe('Creation', () =>
{
    it(
        'Should automatically connect socket on creation by default',
        async () =>
        {
            clientOptions = {
                hostname: '127.0.0.1',
                port    : await randomPort(),
            };

            client = create(clientOptions);

            expect(client.state).toEqual(ClientSocket.CONNECTING);
        },
    );

    it(
        'Should not automatically connect socket if autoConnect is set to false',
        async () =>
        {
            clientOptions = {
                hostname   : '127.0.0.1',
                port       : await randomPort(),
                autoConnect: false,
            };

            client = create(clientOptions);

            expect(client.state).toEqual(ClientSocket.CLOSED);
        },
    );
});
