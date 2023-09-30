import { TGServer } from '../src/server';
import { TGClient } from '../src/client';
import { wait } from './test-util';

let server1: TGServer, server2: TGServer, server3: TGServer, client1: TGClient, client2: TGClient;

describe('Common', () =>
{
    beforeEach(async () =>
    {
        // Client
        client1  = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : 5000,
            }]
        });
        client2  = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : 5001,
            }]
        });
        // Master
        server1 = new TGServer({
            serverName: 'Master',
            port      : 5000,
            peers     : [
                {
                    hostname: '127.0.0.1',
                    port    : 5001,
                },
                {
                    hostname: '127.0.0.1',
                    port    : 5002,
                }
            ],
            log       : {
                enabled: true,
            }
        });
        // Peers
        server2 = new TGServer({
            serverName: 'Peer1',
            port      : 5001,
            log       : {
                enabled: true,
            }
        });
        server3 = new TGServer({
            serverName: 'Peer2',
            port      : 5002,
            log       : {
                enabled: true,
            }
        });
    });
    afterEach(async () =>
    {
        await Promise.all([
            client1.disconnect(),
            client2.disconnect(),
            server1.close(),
            server2.close(),
            server3.close()
        ]);
    });

    it('connect one node to another', async () =>
    {
        await Promise.all([
            server1.waitForPeersAuth(),
            server2.waitForPeersAuth(),
            server3.waitForPeersAuth(),
            client1.waitForConnect(),
            client2.waitForConnect()
        ]);

        await client1
            .get('a')
            .get('a')
            .put('Alice');

        await client2
            .get('a')
            .get('b')
            .put('Bob');

        await wait(1000);
    });
});
