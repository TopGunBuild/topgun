export async function cleanupTasks(client: any, server?: any): Promise<void>
{
    let cleanupTasks = [];
    if (client)
    {
        if (client.state !== 'closed')
        {
            cleanupTasks.push(
                Promise.race([
                    client.listener('disconnect').once(),
                    client.listener('connectAbort').once(),
                ]),
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
            })(),
        );
    }
    await Promise.all(cleanupTasks);
}
