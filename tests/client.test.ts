import { diffCRDT, TGSystemEvent, TGClient, TGUserReference, TGLink } from '../src/client';
import { genString } from './test-util';
import { TGLexLink } from '../src/client/lex-link';

describe('Client', () =>
{
    let client: TGClient;

    beforeEach(() =>
    {
        client = new TGClient();
    });
    afterEach(async () =>
    {
        await client.disconnect();
    });

    it('diffCRDT ', function ()
    {
        const updatedGraph  = {
            'user/said': {
                '_'  : { '#': 'user/said', '>': { 'say': 1683308843720 } },
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

    it('auth wait', async () =>
    {
        const link = client.user().get('some');

        expect(link.waitForAuth()).toBeTruthy();

        // minimum password length 8
        client.user().create('john', '12345678');
        const auth = await client.listener(TGSystemEvent.Auth).once() as TGUserReference;

        expect(auth.alias).toBe('john');
        expect(link.waitForAuth()).toBeFalsy();
    });

    it('minimum password length', async () =>
    {
        try
        {
            await client.user().create('john', genString(7));
        }
        catch (e)
        {
            expect(e.message).toBe(`Minimum password length is 8`);
        }
        const user = await client.user().create('john', genString(8));
        expect(user.alias).toBe('john');

        const client2 = new TGClient({
            passwordMinLength: 3
        });

        try
        {
            await client2.user().create('john', genString(2));
        }
        catch (e)
        {
            expect(e.message).toBe(`Minimum password length is 3`);
        }
        const user2 = await client2.user().create('john', genString(4));
        expect(user2.alias).toBe('john');
    });

    it('maximum password length', async () =>
    {
        try
        {
            await client.user().create('john', genString(49));
        }
        catch (e)
        {
            expect(e.message).toBe(`Maximum password length is 48`);
        }

        const client2 = new TGClient({
            passwordMaxLength: 40
        });

        try
        {
            await client2.user().create('john', genString(41));
        }
        catch (e)
        {
            expect(e.message).toBe(`Maximum password length is 40`);
        }
        const user2 = await client2.user().create('john', genString(38));
        expect(user2.alias).toBe('john');
    });

    it('should array of path', function ()
    {
        const path = client.get('one').get('two').getPath();

        expect(path[0]).toBe('one');
        expect(path[1]).toBe('two');
    });

    it('check link instance', function ()
    {
        const link1 = client.get('chat').get('122');
        const link2 = client.get('chat').get({ '.': { '*': '2019-06-20T' } });

        expect(link1 instanceof TGLink).toBeTruthy();
        expect(link2 instanceof TGLexLink).toBeTruthy();
    });

    it('should get argument', function ()
    {
        try
        {
            client.get(null)
        }
        catch (e)
        {
            expect(e.message).toBe('A non-empty string value and not an underscore is expected.');
        }

        try
        {
            client.get('_')
        }
        catch (e)
        {
            expect(e.message).toBe('A non-empty string value and not an underscore is expected.');
        }

        try
        {
            client.get('')
        }
        catch (e)
        {
            expect(e.message).toBe('A non-empty string value and not an underscore is expected.');
        }
    });

    it('go to parent context on the chain', function ()
    {
        const link = client.get('path').get('deep').get('deeper');
        const deep = link.back();
        const path = link.back(2);
        const root = link.back(3);

        expect(deep instanceof TGLink && deep.getPath().join()).toBe('path,deep');
        expect(path instanceof TGLink && path.getPath().join()).toBe('path');
        expect(root instanceof TGClient).toBeTruthy();
    });
});
