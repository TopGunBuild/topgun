import { isEmptyObject } from '@topgunbuild/typed';
import { diffCRDT, TGClient, TGUserReference, TGLink, TGUserCredentials, TGUserGraph, TGMessage } from '../src/client';
import { genString, wait } from './test-util';
import { TGLexLink } from '../src/client/link/lex-link';
import { getPathData } from '../src/client/graph/graph-utils';

let client: TGClient;

describe('Client', () =>
{
    beforeEach(() =>
    {
        client = new TGClient();
    });
    afterEach(async () =>
    {
        await client.disconnect();
    });

    it('get path ', function ()
    {
        const result = getPathData(['one', 'two', 'three', 'five'], {});

        expect(result).not.toBeUndefined();
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
        const auth = await client.listener('auth').once() as TGUserReference;

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

    it('simple check put/promise/get', async () =>
    {
        const message = await client.get('say').put({ yo: 'hi' });
        const yo      = await client.get('say').get('yo').promise<string>();
        const say     = await client.get('say').promise<{yo: string}>();

        expect(say.yo).toBe('hi');
        expect(yo).toBe('hi');
        expect(message['#']).toBe('say');
        expect(message.err).toBeNull();
        expect(message.ok).toBeTruthy();
        expect(client.graph['_graph']['say'].yo).toBe('hi');
    });

    it('link stream listen and destroy', async () =>
    {
        const link = client.get('chat');

        link.set({ say: 'Hi!' });
        link.set({ say: 'Yeah, man...' });
        link.set({ say: 'Awesome! Call me in 5 minutes..' });
        link.set({ say: '👍' });

        const receivedPackets = [];
        const callback        = (data, id) =>
        {
            receivedPackets.push({ data, id })
        };

        const stream           = link.map().on<{say: string}>(callback);
        const receivedPackets2 = [];

        (async () =>
        {
            for await (const { value, key } of stream)
            {
                receivedPackets2.push({ value, key });
                if (value.say === '👍')
                {
                    stream.destroy();
                }
            }
        })();

        await wait(500);

        stream.destroy();

        expect(isEmptyObject(client.graph['_queries'])).toBeTruthy();
        expect(receivedPackets2.length).toBe(4);
        expect(receivedPackets.length).toBe(4);
    });

    it('lex query', async () =>
    {
        const link = client.get('chat');

        await link.get('2019-06-20T00:00').put({ say: 'one' });
        await link.get('2019-06-20T11:59').put({ say: 'two' });
        await link.get('2019-06-21T00:00').put({ say: 'three' });
        await link.get('2019-06-22T00:00').put({ say: 'four' });

        const stream1 = link
            .prefix('2019-06-20')
            .on<{say: string}>();

        const stream2 = link
            .start('2019-06-20')
            .end('2019-06-22')
            .on<{say: string}>();

        const stream3 = link
            .prefix('2019-06-20')
            .limit(1)
            .reverse()
            .on<{say: string}>();

        const receivedPackets1 = [];
        const receivedPackets2 = [];
        const receivedPackets3 = [];

        (async () =>
        {
            for await (const { value, key } of stream1)
            {
                receivedPackets1.push(key);
                if (value.say === 'two')
                {
                    stream1.destroy();
                }
            }
        })();
        (async () =>
        {
            for await (const { value, key } of stream2)
            {
                receivedPackets2.push(key);
                if (value.say === 'three')
                {
                    stream2.destroy();
                }
            }
        })();
        (async () =>
        {
            for await (const { key } of stream3)
            {
                receivedPackets3.push(key);
                stream3.destroy();
            }
        })();

        await wait(500);

        stream1.destroy();
        stream2.destroy();
        stream3.destroy();

        expect(receivedPackets1.length).toBe(2);
        expect(receivedPackets2.length).toBe(3);
        expect(receivedPackets3[0] === 'chat/2019-06-20T11:59').toBeTruthy();
    });

    it('auth callback', async () =>
    {
        let user: TGUserReference;
        let userFromCallback: TGUserCredentials;
        let userFromListener1: TGUserReference;
        let userFromListener2: TGUserReference;

        await Promise.all([
            (async () =>
            {
                const callback = (value: TGUserCredentials) =>
                {
                    userFromCallback = value;
                };
                user           = await client.user().create('john', genString(20), callback);
            })(),
            (async () =>
            {
                userFromListener1 = await client.listener('auth').once();
            })(),
            (async () =>
            {
                userFromListener2 = await client.listener('auth').once();
            })()
        ]);

        expect(typeof user.pub === 'string').toBeTruthy();
        expect(
            [userFromCallback.pub, userFromListener1.pub, userFromListener2.pub].every(pub => user.pub === pub)
        ).toBeTruthy();
    });

    it('signUp, leave, auth', async () =>
    {
        const password = genString(20);
        const signUp   = await client.user().create('john', password);

        client.user().leave();
        expect(client.user().is).toBeUndefined();

        const signIn = await client.user().auth('john', password);

        expect(
            [signUp.pub, signIn.pub].every(pub => pub === client.user().is.pub)
        ).toBeTruthy();
    });

    it('simple write to user space', async () =>
    {
        await client.user().create('john', genString(20));

        const link = client.user().get('chat');

        link.set({ say: 'Hi!' });
        link.set({ say: 'Yeah, man...' });
        link.set({ say: 'Awesome! Call me in 5 minutes..' });
        link.set({ say: '👍' });

        const stream          = link.map().on<{say: string}>();
        const receivedPackets = [];

        (async () =>
        {
            for await (const { key, value } of stream)
            {
                receivedPackets.push({ key, value });
            }
        })();

        await wait(500);

        stream.destroy();
        expect(receivedPackets.length).toBe(4);
    });

    it('should throw error when name is already in use', async () =>
    {
        try
        {
            await client.user().create('john', '12345678');
            await client.user().create('john', '123456789');
        }
        catch (e)
        {
            expect(e.message).toBe(`Username john is already in use`);
        }
    });

    it('paths should be equal', async () =>
    {
        const user = await client.user().create('john', '12345678');
        await client.user().get('some').put('value');

        const value  = await client.user().get('some').promise<string>();
        const value2 = await client.user(user.pub).get('some').promise<string>();

        const user2 = await client.user(user.pub).promise<TGUserGraph>();

        expect(user.pub === user2.pub).toBeTruthy();
        expect(value === value2).toBeTruthy();
    });
});
