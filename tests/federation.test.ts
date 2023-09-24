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
            appName: 'Master',
            port   : 3458,
            peers  : [
                {
                    hostname: '127.0.0.1',
                    port    : 3459,
                },
                {
                    hostname: '127.0.0.1',
                    port    : 3460,
                }
            ],
            log    : {
                enabled: true,
            }
        });
        // Peers
        server2 = new TGServer({
            appName: 'Peer1',
            port   : 3459,
            log    : {
                enabled: true,
            }
        });
        server3 = new TGServer({
            appName: 'Peer2',
            port   : 3460,
            log    : {
                enabled: true,
            }
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
