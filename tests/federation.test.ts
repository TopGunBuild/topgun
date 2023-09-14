import { TGServer } from '../src/server';
import { TGClient } from '../src/client';
import { wait } from './test-util';

let server1: TGServer, server2: TGServer, server3: TGServer, client: TGClient;

describe('Common', () =>
{
    beforeEach(async () =>
    {
        // Client
        client  = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : 3458,
            }]
        });
        // Master
        server1 = new TGServer({
            port : 3458,
            peers: [
                {
                    hostname: '127.0.0.1',
                    port    : 3459,
                },
                {
                    hostname: '127.0.0.1',
                    port    : 3460,
                }
            ]
        });
        // Peers
        server2 = new TGServer({
            port: 3459
        });
        server3 = new TGServer({
            port: 3460
        });
    });
    afterEach(async () =>
    {
        await Promise.all([
            client.disconnect(),
            server1.close(),
            server2.close(),
            server3.close()
        ]);
    });

    it('connect one node to another', async () =>
    {
        (async () =>
        {
            for await (const { socket } of server1.gateway.listener('connection'))
            {
                // console.log(`server 1 got connection`);
            }
        })();

        (async () =>
        {
            for await (const { socket } of server2.gateway.listener('connection'))
            {
                // console.log(`server 2 got connection`);
            }
        })();

        (async () =>
        {
            for await (const connector of client.listener('connectorConnected'))
            {
                // console.log(`client connector connected`);
            }
        })();

        await Promise.all([
            server1.waitForReady(),
            server2.waitForReady(),
            server3.waitForReady(),
            client.waitForConnect()
        ]);

        // Put to master
        await client
            .get('a')
            .get('b')
            .put('value');

        await wait(1000);
    });
});
