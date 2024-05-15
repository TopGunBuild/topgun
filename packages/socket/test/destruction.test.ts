import { cleanupTasks, randomPort, wait } from '@topgunbuild/test-utils';
import { ClientSocket, create, listen, Socket, SocketServer } from '../src';

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

describe('Socket destruction', () =>
{
    it('Server socket destroy should disconnect the socket', async () =>
    {
        (async () =>
        {
            for await (let { socket } of server.listener('connection'))
            {
                await wait(100);
                socket.disconnect(1000, 'Custom reason');
            }
        })();

        await server.listener('ready').once();

        let { code, reason } = await client.listener('disconnect').once();
        expect(code).toEqual(1000);
        expect(reason).toEqual('Custom reason');
        expect(server.clientsCount).toEqual(0);
        expect(server.pendingClientsCount).toEqual(0);
    });

    it('Server socket destroy should set the active property on the socket to false', async () =>
    {
        let serverSocket: Socket;

        (async () =>
        {
            for await (let { socket } of server.listener('connection'))
            {
                serverSocket = socket;
                expect(socket.state).toEqual('open');
                await wait(100);
                socket.disconnect();
            }
        })();

        await server.listener('ready').once();
        await client.listener('disconnect').once();
        expect(serverSocket.state).toEqual('closed');
    });
});
