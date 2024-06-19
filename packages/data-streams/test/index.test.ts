import { Exchange } from '../src';
import { wait } from '@topgunbuild/test-utils';

it('data stream lifecycle', async () =>
{
    const exchange = new Exchange();

    const destroyed: string[]    = [];
    const subscribed: string[]   = [];
    const unsubscribed: string[] = [];
    const published: any[]     = [];

    const promises: Promise<any>[] = [];

    promises.push(
        (async () =>
        {
            for await (const { streamName } of exchange.listener('destroy'))
            {
                destroyed.push(streamName);
                console.log('destroy')
            }
        })(),
        (async () =>
        {
            for await (const { streamName } of exchange.listener('subscribe'))
            {
                subscribed.push(streamName);
                console.log('subscribe')
            }
        })(),
        (async () =>
        {
            for await (const { streamName } of exchange.listener('unsubscribe'))
            {
                unsubscribed.push(streamName);
                console.log('unsubscribe')
            }
        })()
    );

    const stream = exchange.subscribe<{data: string}>('abc');

    // promises.push(
    //     (async () =>
    //     {
    //         for await (const { data } of stream.listener<{ data: string }>('publish'))
    //         {
    //             published.push(data);
    //         }
    //         exchange.destroy();
    //         exchange.killAllListeners();
    //     })()
    // );
    (async () =>
    {
        for await (const data of stream)
        {
            published.push(data);
            console.log('publish')
        }
    })()

    await stream.publish({data: '123'});
    stream.unsubscribe();
    stream.destroy();

    // exchange.destroy();
    // exchange.killAllListeners();

    // await Promise.all(promises);

    await wait(10);

    console.log(destroyed.length, subscribed.length, unsubscribed.length, published.length);
    expect(stream).not.toBeUndefined()
});

it('should destroy stream after emitting all events', async () =>
{
    const exchange = new Exchange();
    const stream   = exchange.subscribe<string>();
    let destroyed  = false;

    stream.listener('destroy').once().then(() =>
    {
        destroyed = true;
    });

    (async () =>
    {
        for (let i = 0; i < 10; i++)
        {
            await wait(10);
            await stream.publish('world' + i);
        }
        stream.destroy();
    })();

    const receivedPackets: string[] = [];

    await Promise.all([
        (async () =>
        {
            for await (let packet of stream)
            {
                receivedPackets.push(packet);
            }
        })()
    ]);

    expect(destroyed).toBeTruthy();
    expect(receivedPackets.length).toBe(9);
});
