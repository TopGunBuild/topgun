import { AuthToken } from 'topgun-socket/types';
import { TGClient, TGUserGraph } from '../src/client';
import { TGServer } from '../src/server';
import { authenticate } from '../src/sea/authenticate';
import { genString, wait } from './test-util';

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

    it('should client/server authenticate', async () =>
    {
        let serverToken: AuthToken;
        let clientToken: AuthToken;

        (async () =>
        {
            for await (const { socket } of server.gateway.listener('connection'))
            {
                (async () =>
                {
                    for await (let { authToken } of socket.listener('authenticate'))
                    {
                        serverToken = authToken;
                    }
                })();
            }
        })();

        (async () =>
        {
            for await (const connector of client.listener('connectorConnected'))
            {
                (async () =>
                {
                    for await (const { authToken } of connector.client.listener('authenticate'))
                    {
                        clientToken = authToken;
                    }
                })();
            }
        })();

        await Promise.all([
            server.waitForReady(),
            client.waitForConnect()
        ]);

        const user = await client.user().create('john', genString(20));

        expect(clientToken).not.toBeUndefined();
        expect(serverToken).not.toBeUndefined();
        expect(clientToken.pub).toBe(serverToken.pub);
        expect([clientToken.pub, serverToken.pub].every(pub => pub === user.pub)).toBeTruthy();
    });

    it('public keys should equals', async () =>
    {
        const john  = await client.user().create('john', '12345678');
        client.user().leave();
        const billy = await client.user().create('billy', '12345678');
        const john2 = await client.user(john.pub).promise<TGUserGraph>();

        await wait(100);

        expect(john.pub === john2.pub).not.toBeUndefined();
        expect(billy.pub === client.user().is.pub);
    });
});