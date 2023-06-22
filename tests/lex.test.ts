import { createClient, TGClient, TGGraphAdapter, TGGraphData, TGOptionsGet } from '../src/client';
import { TGServer } from '../src/server';
import { graphFromRawValue } from '../src/client/graph/graph-utils';
import { StorageListOptions } from '../src/storage';
import { lexicographicCompare, listFilterMatch } from '../src/storage/utils';

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

const authData = {
    '~XFQj766zXgsSdWe-M3yJ7--BjFmOLq4xOizYjfo40CE.sSGGCKBkmkkv92N1zAxzffyJET0jT-3sh0tIE84SGbg': {
        '_'    : {
            '#': '~XFQj766zXgsSdWe-M3yJ7--BjFmOLq4xOizYjfo40CE.sSGGCKBkmkkv92N1zAxzffyJET0jT-3sh0tIE84SGbg',
            '>': {
                'alias': 1687406907672,
                'auth' : 1687406907672,
                'epub' : 1687406907672,
                'pub'  : 1687406907672
            }
        },
        'alias': '{":":"billy","~":"CoWEkP9DNPe6CnQwR9GB8Er2b1bqucgK22AeTpjjPIhGA5z9mj+hwPGlDTEyPRPrA4NmpbrD9XuRvAsJYuY0sw=="}',
        'auth' : '{":":{"ek":{"ct":"gyyELRpqkMJ3yNdB/4mIRmUrJp/vyh1Zieq+zDQCjMzccKmfVLcqC09v9ldbcl4ZpSRDgusp0OoM4wsrsf9JYCnTfHuh5YqVOoNQCNSdXLfLGnucrluufTBnfEHVpOOxGC56iVakHIlGlyHjSddJYsHvtFujj5TyuquTdQ==","iv":"A4zRO08JQHSYmSIjDZq2","s":"BkpdiNdFp7qh"},"s":"XNVq5WfjTHVoHxDjSERuf1qe9CQ3OHmA6RfxZrO92K4b4qhCCMPp01zvvlmntllU"},"~":"TxDRmXBS5GcaSCWGUCoKg04uBqfh5vY0eLXccOQ21LuQKtOdBJfjlCcIj0iA4WESK6h9aVCBhCs20iy643qLCw=="}',
        'epub' : '{":":"3r08-6U8ju37B_ZZJeObScylod0xEC1tgtn3LSyfBOk.4rmg7RxqkUuzDWKqARSUptpFnY35W1kjMl80A0ugCPw","~":"pM+1fnxgARon47cMlU/t3XeDNW4OE5CkmEuThWfHQ7loFE+lv5ObJiAjQR9WKRazvWlxKHbRcpWfevtvrnARmw=="}',
        'pub'  : 'XFQj766zXgsSdWe-M3yJ7--BjFmOLq4xOizYjfo40CE.sSGGCKBkmkkv92N1zAxzffyJET0jT-3sh0tIE84SGbg'
    }
};

function get(
    opts: TGOptionsGet
): Promise<TGGraphData>
{
    console.log({ opts });
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
        get: (opts?: TGOptionsGet) => get(opts),
        put: (graphData: TGGraphData) => put(graphData),
    };
}

const port = 3457;
let server: TGServer, client: TGClient;

// Run the server and client before start
beforeEach(async () =>
{
    // server = createServer({
    //     port,
    //     adapter: adapter()
    // });
    // await server.waitForReady();
    //
    // client = createClient({
    //     peers: [{
    //         hostname: '127.0.0.1',
    //         port
    //     }]
    // });
    //
    // await client.graph.eachConnector(async connector =>
    // {
    //     await connector.waitForConnection();
    // });

    client = createClient();
});

// Close server and client after each test
afterEach(async () =>
{
    // await client.disconnect();
    // await server.close();
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

    it('should walk', function ()
    {
        const pathArr   = ['widget'];
        const data      = {
            'debug' : 'on',
            'window': {
                'title' : 'Sample Konfabulator Widget',
                'name'  : 'main_window',
                'width' : 500,
                'height': 500
            }
        };
        const graphData = graphFromRawValue(data, pathArr);

        // console.log(
        //     graphData
        // );

        const pathArr2   = ['widget', 'window'];
        const data2      = {
            'title': 'Sample Konfabulator Widget',
        };
        const graphData2 = graphFromRawValue(data, pathArr);

        // expect(graphData).not.toBeUndefined();
        expect(graphData[pathArr2.join('/')])
    });

    it('should walk nested', function ()
    {
        const pathArr   = ['widget', 'window'];
        const data      = {
            'title': 'Sample Konfabulator Widget',
        };
        const graphData = graphFromRawValue(data, pathArr);

        console.log(
            graphData
        );

        expect(graphData).not.toBeUndefined();
    });

    it('should walk nested 2', function ()
    {
        const pathArr   = ['widget', 'window', 'title'];
        const data      = 'Sample Konfabulator Widget';
        const graphData = graphFromRawValue(data, pathArr);

        console.log(
            graphData
        );

        expect(graphData).not.toBeUndefined();
    });

    it('should walk nested 3', function ()
    {
        const pathArr   = ['widget'];
        const data      = {
            'window': {
                'title': 'Sample Konfabulator Widget'
            }
        };
        const graphData = graphFromRawValue(data, pathArr);

        console.log(
            JSON.stringify(graphData, null, 2)
        );

        expect(graphData).not.toBeUndefined();
    });

    it('should walk 4', async () =>
    {
        await client
            .get(`said`)
            .set({
                say: 'Hello'
            })
            .promise();

        console.log(
            JSON.stringify(client.graph['_graph'], null, 2)
        );

        expect(client.graph['_graph']).not.toBeUndefined();
    });

    it('should compare', function ()
    {
        const key                      = 'abc';
        const opts: StorageListOptions = {
            end: 'abc'
        };
        const result                   = listFilterMatch(opts, key);

        console.log(result);

        expect(result).not.toBeUndefined();
    });

    it('should sort keys', function ()
    {
        const replacer = (key, value) =>
            value instanceof Object && !(value instanceof Array) ?
                Object.keys(value)
                    .sort()
                    .reduce((sorted, key) =>
                    {
                        sorted[key] = value[key];
                        return sorted
                    }, {}) :
                value;

        // Usage
        const result = JSON.stringify({ c: 1, a: { d: 0, c: 1, e: { d: '0', a: 0, 1: 4 } } }, replacer);

        console.log(result);

        expect(result).not.toBeUndefined();
    });

    it('should sign graph', () =>
    {
        const fullPath = ['~@billy'];
        const result = graphFromRawValue(authData, fullPath);

        console.log(result);

        expect(result).not.toBeUndefined();
    });
});
