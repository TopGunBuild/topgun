import { cleanupTasks, randomPort, resolveAfterTimeout, wait } from '@topgunbuild/test-utils';
import { Channel, ClientSocket, create, listen, SimpleBroker, Socket, SocketServer, SubscribeOptions } from '../src';

let server: SocketServer, client: ClientSocket;

beforeEach(async () =>
{
    const port = await randomPort();
    server     = listen(port);
    client     = create({
        hostname: '127.0.0.1',
        port,
    });
});

afterEach(async () =>
{
    await cleanupTasks(client, server);
});

describe('Socket pub/sub', () =>
{
    it('Should support subscription batching', async () =>
    {
        (async () =>
        {
            for await (let { socket } of server.listener('connection'))
            {
                let isFirstMessage = true;

                (async () =>
                {
                    for await (let { message } of socket.listener('message'))
                    {
                        if (isFirstMessage)
                        {
                            let data = JSON.parse(message);
                            // All 20 subscriptions should arrive as a single message.
                            expect(data.length).toEqual(20);
                            isFirstMessage = false;
                        }
                    }
                })();
            }
        })();

        let subscribeMiddlewareCounter = 0;

        // Each subscription should pass through the middleware individually, even
        // though they were sent as a batch/array.
        server.addMiddleware(server.MIDDLEWARE_SUBSCRIBE, async (req) =>
        {
            subscribeMiddlewareCounter++;
            expect(req.channel.indexOf('my-channel-')).toEqual(0);
            if (req.channel === 'my-channel-10')
            {
                expect(JSON.stringify(req.data)).toEqual(JSON.stringify({ foo: 123 }));
            }
            else if (req.channel === 'my-channel-12')
            {
                // Block my-channel-12
                let err  = new Error('You cannot subscribe to channel 12');
                err.name = 'UnauthorizedSubscribeError';
                throw err;
            }
        });

        await server.listener('ready').once();

        let channelList: Channel<any>[] = [];
        for (let i = 0; i < 20; i++)
        {
            let subscribeOptions: SubscribeOptions = {
                batch: true,
            };
            if (i === 10)
            {
                subscribeOptions.data = { foo: 123 };
            }
            channelList.push(
                client.subscribe('my-channel-' + i, subscribeOptions),
            );
        }

        (async () =>
        {
            for await (let _ of channelList[12].listener('subscribe'))
            {
                throw new Error('The my-channel-12 channel should have been blocked by MIDDLEWARE_SUBSCRIBE');
            }
        })();

        (async () =>
        {
            for await (let event of channelList[12].listener('subscribeFail'))
            {
                expect(event.error).not.toEqual(null);
                expect(event.error.name).toEqual('UnauthorizedSubscribeError');
            }
        })();

        (async () =>
        {
            for await (let _ of channelList[0].listener('subscribe'))
            {
                client.publish('my-channel-19', 'Hello!');
            }
        })();

        for await (let data of channelList[19])
        {
            expect(data).toEqual('Hello!');
            expect(subscribeMiddlewareCounter).toEqual(20);
            break;
        }
    });

    it('Client should not be able to subscribe to a channel before the handshake has completed', async () =>
    {
        await server.listener('ready').once();

        let isSubscribed = false;
        let error: Error;

        (async () =>
        {
            for await (let _ of server.listener('subscription'))
            {
                isSubscribed = true;
            }
        })();

        // Hack to capture the error without relying on the standard client flow.
        client.transport._callbackMap[2] = {
            event   : '#subscribe',
            data    : { 'channel': 'someChannel' },
            callback: function(err)
            {
                error = err;
            },
        };

        // Trick the server by sending a fake subscribe before the handshake is done.
        client.transport.socket.on('open', () =>
        {
            client.send('{"event":"#subscribe","data":{"channel":"someChannel"},"cid":2}');
        });

        await wait(1000);
        expect(isSubscribed).toEqual(false);
        expect(error).not.toEqual(null);
        expect(error.name).toEqual('InvalidActionError');
    });

    it('Server should be able to handle invalid #subscribe and #unsubscribe and #publish events without crashing', async () =>
    {
        await server.listener('ready').once();

        let nullInChannelArrayError: Error;
        let objectAsChannelNameError: Error;
        let nullChannelNameError: Error;
        let nullUnsubscribeError: Error;

        let undefinedPublishError: Error;
        let objectAsChannelNamePublishError: Error;
        let nullPublishError: Error;

        // Hacks to capture the errors without relying on the standard client flow.
        client.transport._callbackMap[2] = {
            event   : '#subscribe',
            data    : [null],
            callback: function(err)
            {
                nullInChannelArrayError = err;
            },
        };
        client.transport._callbackMap[3] = {
            event   : '#subscribe',
            data    : { 'channel': { 'hello': 123 } },
            callback: function(err)
            {
                objectAsChannelNameError = err;
            },
        };
        client.transport._callbackMap[4] = {
            event   : '#subscribe',
            data    : null,
            callback: function(err)
            {
                nullChannelNameError = err;
            },
        };
        client.transport._callbackMap[5] = {
            event   : '#unsubscribe',
            data    : [null],
            callback: function(err)
            {
                nullUnsubscribeError = err;
            },
        };
        client.transport._callbackMap[6] = {
            event   : '#publish',
            data    : null,
            callback: function(err)
            {
                undefinedPublishError = err;
            },
        };
        client.transport._callbackMap[7] = {
            event   : '#publish',
            data    : { 'channel': { 'hello': 123 } },
            callback: function(err)
            {
                objectAsChannelNamePublishError = err;
            },
        };
        client.transport._callbackMap[8] = {
            event   : '#publish',
            data    : { 'channel': null },
            callback: function(err)
            {
                nullPublishError = err;
            },
        };

        (async () =>
        {
            for await (let _ of client.listener('connect'))
            {
                // Trick the server by sending a fake subscribe before the handshake is done.
                client.send('{"event":"#subscribe","data":[null],"cid":2}');
                client.send('{"event":"#subscribe","data":{"channel":{"hello":123}},"cid":3}');
                client.send('{"event":"#subscribe","data":null,"cid":4}');
                client.send('{"event":"#unsubscribe","data":[null],"cid":5}');
                client.send('{"event":"#publish","data":null,"cid":6}');
                client.send('{"event":"#publish","data":{"channel":{"hello":123}},"cid":7}');
                client.send('{"event":"#publish","data":{"channel":null},"cid":8}');
            }
        })();

        await wait(300);

        expect(nullInChannelArrayError).not.toEqual(null);
        expect(objectAsChannelNameError).not.toEqual(null);
        expect(nullChannelNameError).not.toEqual(null);
        expect(nullUnsubscribeError).not.toEqual(null);
        expect(undefinedPublishError).not.toEqual(null);
        expect(objectAsChannelNamePublishError).not.toEqual(null);
        expect(nullPublishError).not.toEqual(null);
    });

    it('When default SimpleBroker broker engine is used, disconnect event should trigger before unsubscribe event', async () =>
    {
        let eventList = [];

        (async () =>
        {
            await server.listener('ready').once();
            await client.subscribe('foo').listener('subscribe').once();
            await wait(200);
            client.disconnect();
        })();

        let { socket } = await server.listener('connection').once();

        (async () =>
        {
            for await (let event of socket.listener('unsubscribe'))
            {
                eventList.push({
                    type   : 'unsubscribe',
                    channel: event.channel,
                });
            }
        })();

        let disconnectPacket = await socket.listener('disconnect').once();
        eventList.push({
            type  : 'disconnect',
            code  : disconnectPacket.code,
            reason: disconnectPacket.data,
        });

        await wait(0);
        expect(eventList[0].type).toEqual('disconnect');
        expect(eventList[1].type).toEqual('unsubscribe');
        expect(eventList[1].channel).toEqual('foo');
    });

    it('When default SimpleBroker broker engine is used, Server.exchange should support consuming data from a channel', async () =>
    {
        await server.listener('ready').once();

        (async () =>
        {
            await client.listener('connect').once();

            client.publish('foo', 'hi1');
            await wait(10);
            client.publish('foo', 'hi2');
        })();

        let receivedSubscribedData = [];
        let receivedChannelData    = [];

        (async () =>
        {
            let subscription = server.exchange.subscribe('foo');
            for await (let data of subscription)
            {
                receivedSubscribedData.push(data);
            }
        })();

        let channel = server.exchange.channel('foo');
        for await (let data of channel)
        {
            receivedChannelData.push(data);
            if (receivedChannelData.length > 1)
            {
                break;
            }
        }

        await wait(10);

        expect(server.exchange.isSubscribed('foo')).toEqual(true);
        expect(server.exchange.subscriptions().join(',')).toEqual('foo');

        expect(receivedSubscribedData[0]).toEqual('hi1');
        expect(receivedSubscribedData[1]).toEqual('hi2');
        expect(receivedChannelData[0]).toEqual('hi1');
        expect(receivedChannelData[1]).toEqual('hi2');
    });

    it('When default SimpleBroker broker engine is used, Server.exchange should support publishing data to a channel', async () =>
    {
        await server.listener('ready').once();

        (async () =>
        {
            await client.listener('subscribe').once();
            server.exchange.publish('bar', 'hello1');
            await wait(10);
            server.exchange.publish('bar', 'hello2');
        })();

        let receivedSubscribedData = [];
        let receivedChannelData    = [];

        (async () =>
        {
            let subscription = client.subscribe('bar', { data: 123 });
            for await (let data of subscription)
            {
                receivedSubscribedData.push(data);
            }
        })();

        let channel = client.channel('bar');
        for await (let data of channel)
        {
            receivedChannelData.push(data);
            if (receivedChannelData.length > 1)
            {
                break;
            }
        }

        expect(receivedSubscribedData[0]).toEqual('hello1');
        expect(receivedSubscribedData[1]).toEqual('hello2');
        expect(receivedChannelData[0]).toEqual('hello1');
        expect(receivedChannelData[1]).toEqual('hello2');
    });

    it('When disconnecting a socket, the unsubscribe event should trigger after the disconnect event', async () =>
    {
        // Destroy default client/server
        await cleanupTasks(client, server);

        let customBrokerEngine               = new SimpleBroker();
        let defaultUnsubscribeSocket         = customBrokerEngine.unsubscribeSocket;
        customBrokerEngine.unsubscribeSocket = function(socket, channel)
        {
            return resolveAfterTimeout(100, defaultUnsubscribeSocket.call(this, socket, channel));
        };
        const port                           = await randomPort();

        server = listen(port, {
            brokerEngine: customBrokerEngine,
        });

        let eventList = [];

        (async () =>
        {
            await server.listener('ready').once();
            client = create({
                hostname: '127.0.0.1',
                port,
            });

            for await (let _ of client.subscribe('foo').listener('subscribe'))
            {
                (async () =>
                {
                    await wait(200);
                    client.disconnect();
                })();
            }
        })();

        let { socket } = await server.listener('connection').once();

        (async () =>
        {
            for await (let event of socket.listener('unsubscribe'))
            {
                eventList.push({
                    type   : 'unsubscribe',
                    channel: event.channel,
                });
            }
        })();

        let event = await socket.listener('disconnect').once();

        eventList.push({
            type  : 'disconnect',
            code  : event.code,
            reason: event.reason,
        });

        await wait(0);
        expect(eventList[0].type).toEqual('disconnect');
        expect(eventList[1].type).toEqual('unsubscribe');
        expect(eventList[1].channel).toEqual('foo');
    });

    it('Socket should emit an error when trying to unsubscribe to a channel which it is not subscribed to', async () =>
    {
        let errorList: Error[] = [];

        (async () =>
        {
            for await (let { socket } of server.listener('connection'))
            {
                (async () =>
                {
                    for await (let { error } of socket.listener('error'))
                    {
                        errorList.push(error);
                    }
                })();
            }
        })();

        await server.listener('ready').once();

        let error: Error;
        try
        {
            await client.invoke('#unsubscribe', 'bar');
        }
        catch (err)
        {
            error = err;
        }
        expect(error).not.toEqual(null);
        expect(error.name).toEqual('BrokerError');

        await wait(100);
        expect(errorList.length).toEqual(1);
        expect(errorList[0].name).toEqual('BrokerError');
    });

    it('Socket should not receive messages from a channel which it has only just unsubscribed from (accounting for delayed unsubscribe by brokerEngine)', async () =>
    {
        // Destroy default client/server
        await cleanupTasks(client, server);

        let customBrokerEngine               = new SimpleBroker();
        let defaultUnsubscribeSocket         = customBrokerEngine.unsubscribeSocket;
        customBrokerEngine.unsubscribeSocket = function(socket, channel)
        {
            return resolveAfterTimeout(300, defaultUnsubscribeSocket.call(this, socket, channel));
        };
        const port                           = await randomPort();

        server = listen(port, {
            brokerEngine: customBrokerEngine,
        });

        (async () =>
        {
            for await (let { socket } of server.listener('connection'))
            {
                (async () =>
                {
                    for await (let event of socket.listener('unsubscribe'))
                    {
                        if (event.channel === 'foo')
                        {
                            server.exchange.publish('foo', 'hello');
                        }
                    }
                })();
            }
        })();

        await server.listener('ready').once();

        client = create({
            hostname: '127.0.0.1',
            port,
        });
        // Stub the isSubscribed method so that it always returns true.
        // That way the client will always invoke watchers whenever
        // it receives a #publish event.
        client.isSubscribed = () =>
        {
            return true;
        };

        let messageList = [];

        let fooChannel = client.subscribe('foo');

        (async () =>
        {
            for await (let data of fooChannel)
            {
                messageList.push(data);
            }
        })();

        (async () =>
        {
            for await (let _ of fooChannel.listener('subscribe'))
            {
                client.invoke('#unsubscribe', 'foo');
            }
        })();

        await wait(200);
        expect(messageList.length).toEqual(0);
    });

    it('Socket channelSubscriptions and channelSubscriptionsCount should update when socket.kickOut(channel) is called', async () =>
    {
        let errorList        = [];
        let serverSocket;
        let wasKickOutCalled = false;

        (async () =>
        {
            for await (let { socket } of server.listener('connection'))
            {
                serverSocket = socket;

                (async () =>
                {
                    for await (let { error } of socket.listener('error'))
                    {
                        errorList.push(error);
                    }
                })();

                (async () =>
                {
                    for await (let event of socket.listener('subscribe'))
                    {
                        if (event.channel === 'foo')
                        {
                            await wait(50);
                            wasKickOutCalled = true;
                            socket.kickOut('foo', 'Socket was kicked out of the channel');
                        }
                    }
                })();
            }
        })();

        await server.listener('ready').once();

        client.subscribe('foo');

        await wait(100);
        expect(errorList.length).toEqual(0);
        expect(wasKickOutCalled).toEqual(true);
        expect(serverSocket.channelSubscriptionsCount).toEqual(0);
        expect(Object.keys(serverSocket.channelSubscriptions).length).toEqual(0);
    });

    it('Socket channelSubscriptions and channelSubscriptionsCount should update when socket.kickOut() is called without arguments', async () =>
    {
        let errorList: Error[] = [];
        let serverSocket: Socket;
        let wasKickOutCalled   = false;

        (async () =>
        {
            for await (let { socket } of server.listener('connection'))
            {
                serverSocket = socket;

                (async () =>
                {
                    for await (let { error } of socket.listener('error'))
                    {
                        errorList.push(error);
                    }
                })();

                (async () =>
                {
                    for await (let _ of socket.listener('subscribe'))
                    {
                        if (socket.channelSubscriptionsCount === 2)
                        {
                            await wait(50);
                            wasKickOutCalled = true;
                            socket.kickOut();
                        }
                    }
                })();
            }
        })();

        await server.listener('ready').once();

        client.subscribe('foo');
        client.subscribe('bar');

        await wait(200);
        expect(errorList.length).toEqual(0);
        expect(wasKickOutCalled).toEqual(true);
        expect(serverSocket.channelSubscriptionsCount).toEqual(0);
        expect(Object.keys(serverSocket.channelSubscriptions).length).toEqual(0);
    });
});
