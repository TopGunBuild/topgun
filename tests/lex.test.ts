import { createClient, TGClient, TGGraphAdapter, TGGraphData, TGOptionsGet } from '../src/client';
import { createServer, TGServer } from '../src/server';

let state = {
    'xxx': {
        _   : {
            '#': 'xxx',
            '>': {
                name: 1682701808609
            }
        },
        name: 'a'
    },
    'yyy': {
        _   : {
            '#': 'yyy',
            '>': {
                name: 1682701808609
            }
        },
        name: 'b'
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
        peers         : [{
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
        const value = await client
            .get('xxx')
            .start('a')
            .end('b')
            .limit(2)
            .map()
            .promise();

        // console.log(value);
        expect(value['name']).toBe('a');
    });
});
