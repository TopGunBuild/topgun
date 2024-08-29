import { cleanupTasks, randomPort, wait } from '@topgunbuild/test-utils';
import { ClientSocket, create, listen, SocketServer } from '..';

let server: SocketServer, client: ClientSocket;

beforeEach(async () =>
{
    const port = await randomPort();
    server     = listen(port, {
        ackTimeout: 200,
    });
    client     = create({
        hostname  : '127.0.0.1',
        port,
        ackTimeout: 200,
    });

    await server.listener('ready').once();
});

afterEach(async () =>
{
    await cleanupTasks(client, server);
});

describe('Order of events', () =>
{
    it(
        'Should trigger unsubscribe event on channel before disconnect event',
        async () =>
        {
            let hasUnsubscribed = false;
            let fooChannel      = client.subscribe('foo');

            (async () =>
            {
                for await (let _ of fooChannel.listener('subscribe'))
                {
                    await wait(100);
                    client.disconnect();
                }
            })();

            (async () =>
            {
                for await (let _ of fooChannel.listener('unsubscribe'))
                {
                    hasUnsubscribed = true;
                }
            })();

            await client.listener('disconnect').once();
            expect(hasUnsubscribed).toEqual(true);
        },
    );

    it(
        'Should not invoke subscribeFail event if connection is aborted',
        async () =>
        {
            let hasSubscribeFailed    = false;
            let gotBadConnectionError = false;
            let wasConnected          = false;

            (async () =>
            {
                for await (let _ of client.listener('connect'))
                {
                    wasConnected = true;
                    (async () =>
                    {
                        try
                        {
                            await client.invoke('someEvent', 123);
                        }
                        catch (err)
                        {
                            if (err['name'] === 'BadConnectionError')
                            {
                                gotBadConnectionError = true;
                            }
                        }
                    })();

                    let fooChannel = client.subscribe('foo');
                    (async () =>
                    {
                        for await (let _ of fooChannel.listener('subscribeFail'))
                        {
                            hasSubscribeFailed = true;
                        }
                    })();

                    (async () =>
                    {
                        await wait(0);
                        client.disconnect();
                    })();
                }
            })();

            await client.listener('close').once();
            await wait(100);
            expect(wasConnected).toEqual(true);
            expect(gotBadConnectionError).toEqual(true);
            expect(hasSubscribeFailed).toEqual(false);
        },
    );

    it(
        'Should resolve invoke Promise with BadConnectionError after triggering the disconnect event',
        async () =>
        {
            let messageList = [];

            (async () =>
            {
                try
                {
                    await client.invoke('someEvent', 123);
                }
                catch (err)
                {
                    messageList.push({
                        type : 'error',
                        error: err,
                    });
                }
            })();

            (async () =>
            {
                for await (let event of client.listener('disconnect'))
                {
                    messageList.push({
                        type  : 'disconnect',
                        code  : event.code,
                        reason: event.reason,
                    });
                }
            })();

            await client.listener('connect').once();
            client.disconnect();
            await wait(200);
            expect(messageList.length).toEqual(2);
            expect(messageList[0].type).toEqual('disconnect');
            expect(messageList[1].type).toEqual('error');
            expect(messageList[1].error.name).toEqual('BadConnectionError');
        },
    );

    it(
        'Should reconnect if transmit is called on a disconnected socket',
        async () =>
        {
            let fooReceiverTriggered = false;

            (async () =>
            {
                for await (let { socket } of server.listener('connection'))
                {
                    (async () =>
                    {
                        for await (let _ of socket.receiver('foo'))
                        {
                            fooReceiverTriggered = true;
                        }
                    })();
                }
            })();

            let eventList = [];

            (async () =>
            {
                for await (let _ of client.listener('connecting'))
                {
                    eventList.push('connecting');
                }
            })();

            (async () =>
            {
                for await (let _ of client.listener('connect'))
                {
                    eventList.push('connect');
                }
            })();

            (async () =>
            {
                for await (let _ of client.listener('disconnect'))
                {
                    eventList.push('disconnect');
                }
            })();

            (async () =>
            {
                for await (let _ of client.listener('close'))
                {
                    eventList.push('close');
                }
            })();

            (async () =>
            {
                for await (let _ of client.listener('connectAbort'))
                {
                    eventList.push('connectAbort');
                }
            })();

            (async () =>
            {
                await client.listener('connect').once();
                client.disconnect();
                client.transmit('foo', 123);
            })();

            await wait(1000);

            let expectedEventList = ['connect', 'disconnect', 'close', 'connecting', 'connect'];
            expect(JSON.stringify(eventList)).toEqual(JSON.stringify(expectedEventList));
            expect(fooReceiverTriggered).toEqual(true);
        },
    );

    it(
        'Should correctly handle multiple successive connect and disconnect calls',
        async () =>
        {
            let eventList = [];

            (async () =>
            {
                for await (let _ of client.listener('connecting'))
                {
                    eventList.push({
                        event: 'connecting',
                    });
                }
            })();

            (async () =>
            {
                for await (let _ of client.listener('connect'))
                {
                    eventList.push({
                        event: 'connect',
                    });
                }
            })();

            (async () =>
            {
                for await (let event of client.listener('connectAbort'))
                {
                    eventList.push({
                        event : 'connectAbort',
                        code  : event.code,
                        reason: event.reason,
                    });
                }
            })();

            (async () =>
            {
                for await (let event of client.listener('disconnect'))
                {
                    eventList.push({
                        event : 'disconnect',
                        code  : event.code,
                        reason: event.reason,
                    });
                }
            })();

            (async () =>
            {
                for await (let event of client.listener('close'))
                {
                    eventList.push({
                        event : 'close',
                        code  : event.code,
                        reason: event.reason,
                    });
                }
            })();

            client.disconnect(1000, 'One');
            client.connect();
            client.disconnect(4444, 'Two');

            (async () =>
            {
                await client.listener('connect').once();
                client.disconnect(4455, 'Three');
            })();

            client.connect();

            await wait(200);

            let expectedEventList = [
                {
                    event : 'connectAbort',
                    code  : 1000,
                    reason: 'One',
                },
                {
                    event : 'close',
                    code  : 1000,
                    reason: 'One',
                },
                {
                    event: 'connecting',
                },
                {
                    event : 'connectAbort',
                    code  : 4444,
                    reason: 'Two',
                },
                {
                    event : 'close',
                    code  : 4444,
                    reason: 'Two',
                },
                {
                    event: 'connecting',
                },
                {
                    event: 'connect',
                },
                {
                    event : 'disconnect',
                    code  : 4455,
                    reason: 'Three',
                },
                {
                    event : 'close',
                    code  : 4455,
                    reason: 'Three',
                },
            ];
            expect(JSON.stringify(eventList)).toEqual(JSON.stringify(expectedEventList));
        },
    );
});
