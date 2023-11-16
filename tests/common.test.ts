import { AuthToken } from '@topgunbuild/socket/types';
import { TGClient } from '../src/client';
import { TGServer } from '../src/server';
import { authenticate } from '../src/sea/authenticate';
import { genString, wait } from './test-util';
import { flattenGraphData } from '../src/client/graph/graph-utils';
import { filterMatch } from '../src/storage';
import { TGUserGraph, TGValue } from '../src/types';

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

    it('reference', async () =>
    {
        await Promise.all([
            server.waitForReady(),
            client.waitForConnect()
        ]);

        const node1 = { name: 'Node 1' };
        const node2 = { name: 'Node 2' };

        await client.get('nodes').get('node').put(node1);
        await client.get('events').get('reference').put({
            '#': 'nodes/node'
        });

        const eventStream     = client.get('events').get('reference').on();
        const receivedPackets = [];

        (async () =>
        {
            for await (const { value } of eventStream)
            {
                receivedPackets.push(value);
            }
        })();

        await client.get('nodes').get('node').put(node2);

        await wait(50);
        eventStream.destroy();

        expect(receivedPackets[0].name).toBe(node1.name);
        expect(receivedPackets[1].name).toBe(node2.name);
    });

    // it('edge', async () =>
    // {
    //     const node1 = { name: 'Node 1' };
    //     const node2 = { name: 'Node 2' };
    //
    //     const node1Meta = await client.get('nodes').set(node1);
    //     const node2Meta = await client.get('nodes').set(node2);
    //
    //     await client.get('events').set(node1Meta);
    //
    //     await wait(50);
    //
    //     const eventStream     = client.get('events').collection().on();
    //     const receivedPackets = [];
    //
    //     (async () =>
    //     {
    //         for await (const { value, key } of eventStream)
    //         {
    //             console.log(key, value);
    //             receivedPackets.push({ value, key });
    //         }
    //     })();
    //
    //     await wait(50);
    //     await client.get('events').set(node2Meta);
    //     await wait(50);
    //
    //     expect(receivedPackets.length).not.toBe(0);
    // });

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

        await wait(50);

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
        const stream = client.user().get('deep').collection().on();

        const receivedPackets: {value: TGValue, key: string}[] = [];

        (async () =>
        {
            for await (const { value, key } of stream)
            {
                // console.log(key, value);
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

        // console.log(
        //     client.graph.state
        // );

        expect(typeof receivedPackets[0].value === 'object').toBeTruthy();
        expect(receivedPackets[1].value === null).toBeTruthy();

        const soul = `~${user.pub}/deep/deeper`;

        expect(receivedPackets[0].key === soul).toBeTruthy();
        expect(receivedPackets[1].key === soul).toBeTruthy();
    });

    it('filterMatch', () =>
    {
        const result1 = filterMatch(
            '~S8clCMII_u1YwuUeKbGEWCCpknfa8xt9jvPaOWmhgBo.x5EWl2pw3j1rd1RoUSIf4-iQD_QMrX-qEpYnvydiOYI', {
                '#': '~S8clCMII_u1YwuUeKbGEWCCpknfa8xt9jvPaOWmhgBo.x5EWl2pw3j1rd1RoUSIf4-iQD_QMrX-qEpYnvydiOYI/who/said'
            }
        );
        expect(result1).toBeFalsy();

        const result2 = filterMatch(
            '~S8clCMII_u1YwuUeKbGEWCCpknfa8xt9jvPaOWmhgBo.x5EWl2pw3j1rd1RoUSIf4-iQD_QMrX-qEpYnvydiOYI', {
                '#': '~S8clCMII_u1YwuUeKbGEWCCpknfa8xt9jvPaOWmhgBo.x5EWl2pw3j1rd1RoUSIf4-iQD_QMrX-qEpYnvydiOYI'
            }
        );
        expect(result2).toBeTruthy();

        const result3 = filterMatch(
            '~S8clCMII_u1YwuUeKbGEWCCpknfa8xt9jvPaOWmhgBo.x5EWl2pw3j1rd1RoUSIf4-iQD_QMrX-qEpYnvydiOYI/who/said', {
                '*': '~S8clCMII_u1YwuUeKbGEWCCpknfa8xt9jvPaOWmhgBo.x5EWl2pw3j1rd1RoUSIf4-iQD_QMrX-qEpYnvydiOYI/'
            }
        );
        expect(result3).toBeTruthy();
    });
});