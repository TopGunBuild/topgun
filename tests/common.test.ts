import { AuthToken } from 'topgun-socket/types';
import { TGClient } from '../src/client';
import { TGServer } from '../src/server';
import { genString, loginBob, passwordBob } from './test-util';

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

        const user = await client.user().create(loginBob, passwordBob);

        console.log({user,clientToken});

        expect(clientToken).not.toBeUndefined();
        expect(serverToken).not.toBeUndefined();
        expect(clientToken.pub).toBe(serverToken.pub);
        expect([clientToken.pub, serverToken.pub].every(pub => pub === user.pub)).toBeTruthy();
    });
});