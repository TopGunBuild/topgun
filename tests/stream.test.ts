import { TGExchange } from '../src/stream/exchange';
import { wait } from './test-util';

describe('Stream', () =>
{
    it('should destroy stream after emitting all events', async () =>
    {
        const exchange = new TGExchange();
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
                await exchange.publish(stream.name, 'world' + i);
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

    it('subscribe to only part of the events', async () =>
    {
        const exchange = new TGExchange();
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
            })()
        ]);

        expect(receivedPackets.length).toBe(5);
    });

    it('the stream can have multiple subscriptions', async () =>
    {
        const exchange = new TGExchange();
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
            })()
        ]);

        expect(receivedPackets.length).toBe(27);
    });
});