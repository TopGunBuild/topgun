import { cleanupTasks, randomPort } from '@topgunbuild/test-utils';
import { ClientSocket, create, listen, SocketServer } from '../src';

let server: SocketServer, client: ClientSocket;

beforeEach(async () =>
{
    const port = await randomPort();
    server = listen(port);
    client = create({
        hostname: '127.0.0.1',
        port
    });
});

afterEach(async () =>
{
    await cleanupTasks(client, server);
});

describe('Socket handshake', () =>
{
    it('Exchange is attached to socket before the handshake event is triggered', async () =>
    {
        await server.listener('ready').once();
        let { socket } = await server.listener('handshake').once();
        expect(socket.exchange).not.toEqual(null);
    });
});
