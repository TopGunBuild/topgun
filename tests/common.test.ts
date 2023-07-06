import { TGClient } from '../src/client';
import { TGServer } from '../src/server';
import { genString } from './test-util';

const PORT_NUMBER = 3457;
let server: TGServer, client: TGClient;

describe('Common', () =>
{
    beforeEach(async () =>
    {
        client = new TGClient({
            peers: [{
                hostname: '127.0.0.1',
                port    : PORT_NUMBER,
            }]
        });
        server = new TGServer({
            port: PORT_NUMBER
        });
    });
    afterEach(async () =>
    {
        await Promise.all([
            client.disconnect(),
            server.close()
        ]);
    });

    it('should ', async () =>
    {
        let serverToken;

        (async () =>
        {
            for await (const { socket } of server.server.listener('connection'))
            {
                (async () =>
                {
                    for await (let { authToken } of socket.listener('authenticate'))
                    {
                        serverToken = authToken;
                    }
                })();
            }

            // for await
        })();

        await Promise.all([
            server.waitForReady(),
            client.waitForConnect()
        ]);

        await client.user().create('john', genString(20));

        // console.log(client.connectors()[0]);

        expect(!!serverToken).toBeTruthy();
    });
});