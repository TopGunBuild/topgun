import { cleanupTasks, randomPort, wait } from '@topgunbuild/test-utils';
import { ClientSocket, create, listen, SocketServer } from '../src';

let server: SocketServer, client: ClientSocket;

beforeEach(async () =>
{
    const port = await randomPort();
    server = listen(port, {
        wsEngine: 'ws'
    });
    client = create({
        hostname: '127.0.0.1',
        port
    });
});

afterEach(async () =>
{
    await cleanupTasks(client, server);
});

describe('Socket disconnection', () =>
{
    it('Server-side socket disconnect event should trigger if the socket completed the handshake (not connectAbort)', async () =>
    {
        let connectionOnServer = false;

        (async () =>
        {
            for await (let { } of server.listener('connection'))
            {
                connectionOnServer = true;
            }
        })();

        await server.listener('ready').once();

        let socketDisconnected              = false;
        let socketDisconnectedBeforeConnect = false;
        let clientSocketAborted             = false;

        (async () =>
        {
            let { socket } = await server.listener('handshake').once();
            expect(server.pendingClientsCount).toEqual(1);
            expect(server.pendingClients[socket.id]).not.toEqual(null);

            (async () =>
            {
                let event = await socket.listener('disconnect').once();
                if (!connectionOnServer)
                {
                    socketDisconnectedBeforeConnect = true;
                }
                socketDisconnected = true;
                expect(event.code).toEqual(4445);
                expect(event.reason).toEqual('Disconnect after handshake');
            })();

            (async () =>
            {
                await socket.listener('connectAbort').once();
                clientSocketAborted = true;
            })();
        })();

        let serverDisconnected  = false;
        let serverSocketAborted = false;

        (async () =>
        {
            await server.listener('disconnection').once();
            serverDisconnected = true;
        })();

        (async () =>
        {
            await server.listener('connectionAbort').once();
            serverSocketAborted = true;
        })();

        await wait(200);
        client.disconnect(4445, 'Disconnect after handshake');

        await wait(1000);

        expect(socketDisconnectedBeforeConnect).toEqual(false);
        expect(socketDisconnected).toEqual(true);
        expect(clientSocketAborted).toEqual(false);
        expect(serverDisconnected).toEqual(true);
        expect(serverSocketAborted).toEqual(false);
    });

    it('The close event should trigger when the socket loses the connection before the handshake', async () =>
    {
        await server.listener('ready').once();

        let serverSocketClosed  = false;
        let serverClosure       = false;

        (async () =>
        {
            for await (let { socket } of server.listener('handshake'))
            {
                let event          = await socket.listener('close').once();
                serverSocketClosed = true;
                expect(event.code).toEqual(4444);
                expect(event.reason).toEqual('Disconnect before handshake');
            }
        })();

        (async () =>
        {
            for await (let event of server.listener('closure'))
            {
                expect(event.socket.state).toEqual(event.socket.CLOSED);
                serverClosure = true;
            }
        })();

        await wait(100);
        client.disconnect(4444, 'Disconnect before handshake');

        await wait(1000);
        expect(serverSocketClosed).toEqual(true);
        expect(serverClosure).toEqual(true);
    });

    it('The close event should trigger when the socket loses the connection after the handshake', async () =>
    {
        await server.listener('ready').once();

        let serverSocketClosed       = false;
        let serverSocketDisconnected = false;
        let serverClosure            = false;

        (async () =>
        {
            for await (let { socket } of server.listener('handshake'))
            {
                let event          = await socket.listener('close').once();
                serverSocketClosed = true;
                expect(event.code).toEqual(4445);
                expect(event.reason).toEqual('Disconnect after handshake');
            }
        })();

        (async () =>
        {
            for await (let _ of server.listener('disconnection'))
            {
                serverSocketDisconnected = true;
            }
        })();

        (async () =>
        {
            for await (let event of server.listener('closure'))
            {
                expect(event.socket.state).toEqual(event.socket.CLOSED);
                serverClosure = true;
            }
        })();

        await wait(100);
        client.disconnect(4445, 'Disconnect after handshake');

        await wait(1000);
        expect(serverSocketClosed).toEqual(true);
        expect(serverSocketDisconnected).toEqual(true);
        expect(serverClosure).toEqual(true);
    });
});
