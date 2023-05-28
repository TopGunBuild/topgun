import { diffCRDT, TGClient } from '../src/client';

describe('Client', () =>
{
    const client = new TGClient();

    it('diffCRDT ', function ()
    {
        const updatedGraph  = {
            'user/said': {
                '_': { '#': 'user/said', '>': { 'say': 1683308843720 } },
                'say': 'Hello'
            }
        };
        const existingGraph = {
            'user/said': {
                '_'  : { '#': 'user/said', '>': { 'say': 1683308843720 } },
                'say': 'Hello'
            }
        };

        const diff = diffCRDT(updatedGraph, existingGraph);

        expect(diff).toBeUndefined();
    });

    it('callback', async () =>
    {
        const key = 'test';

        client.get(key).put({ yo: 'hi' }, async (ack) =>
        {
            expect(ack['#']).toBe(key);
            expect(ack.ok).toBeTruthy();
            expect(client.graph['_graph'][key]?.yo).toBe('hi');
        });
    });

    it('save/read number', async () =>
    {
        client
            .get('a')
            .get('b')
            .put(0, (ack) =>
            {
                expect(ack.err).toBeFalsy();
                client
                    .get('a')
                    .get('b')
                    .once((data) =>
                    {
                        expect(data).toBe(0);
                    });
            });
    });

    it('save/read json', async () =>
    {
        client
            .get('a')
            .get('c')
            .put(JSON.stringify({ hello: 'world' }), (ack) =>
            {
                expect(ack.err).toBeFalsy();
                client
                    .get('a')
                    .get('c')
                    .once((data) =>
                    {
                        expect(data).toBe(JSON.stringify({ hello: 'world' }));
                    });
            });
    });

    it('read once', async () =>
    {
        const results: string[] = [];
        client.get('key').put({
            y: 1,
        });
        client.get('key').once((ack) =>
        {
            results.push(JSON.stringify(ack));
        });

        jest.setTimeout(500);
        client.get('key').once((ack) =>
        {
            results.push(JSON.stringify(ack));
        });

        jest.setTimeout(1000);
        client.get('key').once((ack) =>
        {
            results.push(JSON.stringify(ack));
            expect(results.length).toBe(3);
            expect(results).toContain(JSON.stringify(ack));
        });
    });
});
