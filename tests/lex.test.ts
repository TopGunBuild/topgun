import { TGClient, TGGraphAdapter, TGGraphData, TGOptionsGet } from '../src/client';
import { createServer } from '../src/server';

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

beforeEach(async () =>
{

});

describe('LEX', () =>
{
    it('test lex query', async () =>
    {
        const client = new TGClient({
            peers         : [`http://127.0.0.1:${port}`],
            persistSession: false
        });
        const server = createServer({
            port,
            adapter: adapter()
        });

        client.get('xxx').put({
            value: 'yyy'
        });

        client
            .get('xxx')
            .once((data, id) =>
            {
                console.log(data);
                expect(client).not.toBeUndefined();
            });

        /* client
             .get('xxx')
             .start('a')
             .end('b')
             .limit(2)
             .map()
             .once((data, id) =>
             {
                 console.log(data);
                 expect(client).not.toBeUndefined();
             });*/
    });
});
