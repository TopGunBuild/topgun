import { AuthToken } from '@topgunbuild/socket/types';
import { TGClient, TGUserGraph, TGValue } from '../src/client';
import { TGServer } from '../src/server';
import { authenticate } from '../src/sea/authenticate';
import { genString, wait } from './test-util';
import { flattenGraphData } from '../src/client/graph/graph-utils';
import { queryOptionsFromGetOptions } from '../src/storage';

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
            port: PORT_NUMBER,
            log : {
                levels: ['log']
            }
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
        const john = await client.user().create('john', '12345678');
        client.user().leave();
        const billy = await client.user().create('billy', '12345678');
        const john2 = await client.user(john.pub).promise<TGUserGraph>();

        await wait(100);

        expect(john.pub === john2.pub).not.toBeUndefined();
        expect(billy.pub === client.user().is.pub);
    });

    it('flat graph data', async () =>
    {
        const result1 = flattenGraphData(null, ['zzz']);
        const result2 = flattenGraphData({ value: { yyy: 123 } }, ['zzz']);

        expect(JSON.stringify(result1.graphData)).toBe('{"zzz":null}');
        expect(JSON.stringify(result2.graphData)).toBe('{"zzz":{"value":{"#":"zzz/value"}},"zzz/value":{"yyy":123}}');
    });

    it('delete node', async () =>
    {
        const user   = await client.user().create('billy', '12345678');
        const stream = client.user().get('deep').map().on();

        const receivedPackets: {value: TGValue, key: string}[] = [];

        (async () =>
        {
            for await (const { value, key } of stream)
            {
                receivedPackets.push({ value, key });
            }
        })();

        await client.user()
            .get('deep')
            .get('deeper')
            .put({
                value: true
            });

        await wait(50);

        await client.user()
            .get('deep')
            .get('deeper')
            .remove();

        await wait(50);

        stream.destroy();

        expect(typeof receivedPackets[0].value === 'object').toBeTruthy();
        expect(receivedPackets[1].value === null).toBeTruthy();

        const soul = `~${user.pub}/deep/deeper`;

        expect(receivedPackets[0].key === soul).toBeTruthy();
        expect(receivedPackets[1].key === soul).toBeTruthy();
    });

    it('Get options to query options', () =>
    {
        const notList = queryOptionsFromGetOptions({
            '#': 'soul',
            // '%': 200,
            // '.': {
            //     '*': 'my-prefix'
            // }
        });

        expect(soulOnly).toBeNull();
    });
});