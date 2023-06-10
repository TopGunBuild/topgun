import { createClient, TGClient, TGGraphAdapter, TGGraphData, TGOptionsGet } from '../src/client';
import { createServer, TGServer } from '../src/server';

let state = {
    'chat'                         : {
        '_'                       : {
            '#': 'chat',
            '>': {
                '2019-06-20T06:49:08.348Z': 1686379748349,
                '2019-06-20T06:50:28.594Z': 1686379828595,
                '2019-06-21T07:37:24.197Z': 1686382644198,
                '2019-06-22T07:37:50.550Z': 1686382670551
            }
        },
        '2019-06-20T06:49:08.348Z': { '#': 'chat/2019-06-20T06:49:08.348Z' },
        '2019-06-20T06:50:28.594Z': { '#': 'chat/2019-06-20T06:50:28.594Z' },
        '2019-06-21T07:37:24.197Z': { '#': 'chat/2019-06-21T07:37:24.197Z' },
        '2019-06-22T07:37:50.550Z': { '#': 'chat/2019-06-22T07:37:50.550Z' }
    },
    'chat/2019-06-20T06:49:08.348Z': {
        '_'         : {
            '#': 'chat/2019-06-20T06:49:08.348Z',
            '>': { 'message': 1686379748349 }
        }, 'message': '2023-06-10T06:49:08.349Z'
    },
    'chat/2019-06-20T06:50:28.594Z': {
        '_'         : {
            '#': 'chat/2019-06-20T06:50:28.594Z',
            '>': { 'message': 1686379828595 }
        }, 'message': '2023-06-10T06:50:28.594Z'
    },
    'chat/2019-06-21T07:37:24.197Z': {
        '_'         : {
            '#': 'chat/2019-06-21T07:37:24.197Z',
            '>': { 'message': 1686379828595 }
        }, 'message': '2023-06-10T06:50:28.594Z'
    },
    'chat/2019-06-22T07:37:50.550Z': {
        '_'         : {
            '#': 'chat/2019-06-22T07:37:50.550Z',
            '>': { 'message': 1686382670551 }
        }, 'message': '2023-06-10T07:37:50.551Z'
    }
};

function get(
    nodeSoul: string,
    opts?: TGOptionsGet
): Promise<TGGraphData>
{
    console.log({ nodeSoul, opts });
    return Promise.resolve(state);
}

function put(data: TGGraphData): Promise<TGGraphData>
{
    state = { ...state, ...data };
    return Promise.resolve(state);
}

function adapter(): TGGraphAdapter
{
    return {
        get: (soul: string, opts?: TGOptionsGet) => get(soul, opts),
        put: (graphData: TGGraphData) => put(graphData),
    };
}

const port = 3457;
let server: TGServer, client: TGClient;

// Run the server and client before start
beforeEach(async () =>
{
    server = createServer({
        port,
        adapter: adapter()
    });
    await server.waitForReady();

    client = createClient({
        peers: [{
            hostname: '127.0.0.1',
            port
        }]
    });

    await client.graph.eachConnector(async connector =>
    {
        await connector.waitForConnection();
    });
});

// Close server and client after each test
afterEach(async () =>
{
    await client.disconnect();
    await server.close();
});

describe('LEX', () =>
{
    it('test lex query', async () =>
    {
        const query = client
            .get('xxx')
            .start('a')
            .end('b')
            .limit(2);

        const value = await query
            .map()
            .promise();

        console.log(query.toString());
        expect(value).toBeUndefined();
    });
});
