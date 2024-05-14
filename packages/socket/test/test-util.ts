import { ClientSocket, SocketServer } from '../src';

export function wait(duration = 0): Promise<void>
{
    return new Promise((resolve) =>
    {
        setTimeout(() => resolve(), duration);
    });
}

export async function cleanupTasks(client: ClientSocket, server?: SocketServer): Promise<void>
{
    let cleanupTasks = [];
    if (client)
    {
        if (client.state !== ClientSocket.CLOSED)
        {
            cleanupTasks.push(
                Promise.race([
                    client.listener('disconnect').once(),
                    client.listener('connectAbort').once()
                ])
            );
            client.disconnect();
        }
        else
        {
            client.disconnect();
        }
    }
    if (server)
    {
        cleanupTasks.push(
            (async () =>
            {
                server.httpServer.close();
                await server.close();
            })()
        );
    }
    await Promise.all(cleanupTasks);
}
