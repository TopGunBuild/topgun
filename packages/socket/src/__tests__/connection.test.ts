import { cleanupTasks, randomPort, wait } from '@topgunbuild/test-utils';
import { create, EventObject, listen } from '..';

it('Server-side socket connect event and server connection event should trigger', async () =>
{
    const port   = await randomPort();
    const server = listen(port);

    let connectionEmitted = false;
    let connectionEvent: EventObject;

    (async () =>
    {
        for await (let event of server.listener('connection'))
        {
            connectionEvent   = event;
            connectionEmitted = true;
        }
    })();

    await server.listener('ready').once();

    const client = create({
        hostname: '127.0.0.1',
        port,
    });

    let connectEmitted = false;
    let connectStatus: EventObject;
    let socketId: string;

    (async () =>
    {
        for await (let { socket } of server.listener('handshake'))
        {
            (async () =>
            {
                for await (let serverSocketStatus of socket.listener('connect'))
                {
                    socketId               = socket.id;
                    connectEmitted         = true;
                    connectStatus          = serverSocketStatus;
                    // This is to ensure that a status change on the server does not affect the status sent to the client.
                    serverSocketStatus.foo = 123;
                }
            })();
        }
    })();

    let clientConnectEmitted             = false;
    let clientConnectStatus: EventObject = null;

    (async () =>
    {
        for await (let event of client.listener('connect'))
        {
            clientConnectEmitted = true;
            clientConnectStatus  = event;
        }
    })();

    await wait(50);
    await cleanupTasks(client, server);

    expect(connectEmitted).toEqual(true);
    expect(connectionEmitted).toEqual(true);
    expect(clientConnectEmitted).toEqual(true);

    expect(connectionEvent).not.toEqual(null);
    expect(connectionEvent.id).toEqual(socketId);
    expect(connectionEvent.pingTimeout).toEqual(server.pingTimeout);

    expect(connectStatus).not.toEqual(null);
    expect(connectStatus.id).toEqual(socketId);
    expect(connectStatus.pingTimeout).toEqual(server.pingTimeout);

    expect(clientConnectStatus).not.toEqual(null);
    expect(clientConnectStatus.id).toEqual(socketId);
    expect(clientConnectStatus.pingTimeout).toEqual(server.pingTimeout);
    expect(clientConnectStatus['foo']).toBeUndefined();
});
