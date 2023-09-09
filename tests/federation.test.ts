import { TGServer } from '../src/server';
import { TGClient } from '../src/client';

let server1: TGServer, server2: TGServer, client: TGClient;

describe('Common', () =>
{
    beforeEach(async () =>
    {
        client  = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : 3458,
            }]
        });
        server1 = new TGServer({
            port : 3458,
            peers: [{
                hostname: '127.0.0.1',
                port    : 3459,
            }]
        });
        server2 = new TGServer({
            port: 3459
        });
    });
    afterEach(async () =>
    {
        await Promise.all([
            client.disconnect(),
            server1.close(),
            server2.close()
        ]);
    });

    it('connect one node to another', async () =>
    {
        (async () =>
        {
            for await (const { socket } of server1.gateway.listener('connection'))
            {
                console.log(`server 1 got connection`);
            }
        })();

        (async () =>
        {
            for await (const { socket } of server2.gateway.listener('connection'))
            {
                console.log(`server 2 got connection`);
            }
        })();

        (async () =>
        {
            for await (const connector of client.listener('connectorConnected'))
            {
                console.log(`client connector connected`);
            }
        })();

        await Promise.all([
            server1.waitForReady(),
            server2.waitForReady(),
            client.waitForConnect()
        ]);

        await client
            .get('a')
            .get('b')
            .put('value');
    })
});
