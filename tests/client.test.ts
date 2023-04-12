import { TGClient } from '../src/client';
import { dataWalking } from '../src/utils/data-walking';

describe('Client', () => {
    const client = new TGClient();

    it('callback', async () => {
        const key = 'test';

        client.get(key).put({ yo: 'hi' }, async (ack) => {
            expect(ack['#']).toBe(key);
            expect(ack.ok).toBeTruthy();
            expect(client.graph['_graph'][key]?.yo).toBe('hi');
        });
    });

    it('save/read number', async () => {
        client
            .get('a')
            .get('b')
            .put(0, (ack) => {
                expect(ack.err).toBeFalsy();
                console.log(ack);
                client
                    .get('a')
                    .get('b')
                    .once((data) => {
                        console.log(client.graph['_graph']);
                        expect(data).toBe(0);
                    });
            });
    });

    it('save/read json', async () => {
        client
            .get('a')
            .get('c')
            .put(JSON.stringify({ hello: 'world' }), (ack) => {
                expect(ack.err).toBeFalsy();
                client
                    .get('a')
                    .get('c')
                    .once((data) => {
                        expect(data).toBe(JSON.stringify({ hello: 'world' }));
                    });
            });
    });

    it('read once', async () => {
        const results: string[] = [];
        client.get('key').put({
            y: 1,
        });
        client.get('key').once((ack) => {
            results.push(JSON.stringify(ack));
        });
        setTimeout(() => {
            client.get('key').once((ack) => {
                results.push(JSON.stringify(ack));
            });
        }, 500);
        setTimeout(() => {
            client.get('key').once((ack) => {
                results.push(JSON.stringify(ack));
                expect(results.length).toBe(3);
                expect(results).toContain(JSON.stringify(ack));
            });
        }, 1000);
    });

    it('prepare input data', async () => {
        const data = {
            y: {
                name: 'Billy',
                email: 'billy@minigun.tech',
                bio: {
                    date: '27.11.85',
                },
            },
            w: 'wall',
        };

        const a = dataWalking(data, ['x', 'z']);
        console.log(a);
        // const b = addMissingState(a);
        // const c = flattenGraphData(addMissingState(a));
    });
});
