import { Exchange } from '..';
import { wait } from '@topgunbuild/test-utils';

describe('Data streams', () =>
{
    it('data stream lifecycle', async () =>
    {
        const exchange = new Exchange();

        let destroyed: boolean    = false;
        let subscribed: boolean   = false;
        let unsubscribed: boolean = false;
        let published: boolean    = false;

        exchange.listener('destroy').once().then(() =>
        {
            destroyed = true;
        });
        exchange.listener('unsubscribe').once().then(() =>
        {
            unsubscribed = true;
        });
        exchange.listener('subscribe').once().then(() =>
        {
            subscribed = true;
        });

        const stream = exchange.subscribe<string>();

        (async () =>
        {
            for await (const _ of stream)
            {
                published = true;
                stream.unsubscribe();
                stream.destroy();
            }
        })();

        await stream.publish('123');
        await wait(0);

        expect(subscribed).toBeTruthy();
        expect(published).toBeTruthy();
        expect(unsubscribed).toBeTruthy();
        expect(destroyed).toBeTruthy();
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
            })(),
        ]);

        expect(destroyed).toBeTruthy();
        expect(receivedPackets.length).toBe(9);
    });

    it('subscribe to only part of the events', async () =>
    {
        const exchange = new Exchange();
        const stream   = exchange.subscribe();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);

                if (i === 3)
                {
                    stream.unsubscribe();
                }
                else if (i === 7)
                {
                    stream.subscribe();
                }
                await stream.publish('world' + i);
            }
            stream.destroy();
        })();

        const receivedPackets = [];

        for await (let packet of stream)
        {
            receivedPackets.push(packet);
        }

        expect(receivedPackets.length).toBe(5);
    });

    it('the stream can have multiple subscriptions', async () =>
    {
        const exchange = new Exchange();
        const stream   = exchange.subscribe();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                await exchange.publish(stream.name, 'world' + i);
            }
            stream.destroy();
        })();

        const receivedPackets = [];

        await Promise.all([
            (async () =>
            {
                for await (let packet of stream)
                {
                    receivedPackets.push(packet);
                }
            })(),
            (async () =>
            {
                for await (let packet of stream)
                {
                    receivedPackets.push(packet);
                }
            })(),
            (async () =>
            {
                for await (let packet of stream)
                {
                    receivedPackets.push(packet);
                }
            })(),
        ]);

        expect(receivedPackets.length).toBe(27);
    });
});
