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

        await Promise.all([
            server.waitForReady(),
            client.waitForConnect()
        ]);
    });
    afterEach(async () =>
    {
        await Promise.all([
            // client.disconnect(),
            server.close()
        ]);
    });

    it('should ', async () =>
    {
        const user = await client.user().create('john', genString(20));

        expect(!!user).toBeTruthy();
    });
});