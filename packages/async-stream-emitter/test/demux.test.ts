import { StreamDemux } from '../src/stream-demux';
import { wait } from '@topgunbuild/test-utils';


describe('StreamDemux', () =>
{
    it('should demultiplex packets over multiple substreams', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                demux.write('hello', 'world' + i);
                demux.write('abc', 'def' + i);
            }
            demux.close('hello');
            demux.close('abc');
        })();

        let receivedHelloPackets = [];
        let receivedAbcPackets   = [];

        await Promise.all([
            (async () =>
            {
                let substream = demux.stream('hello');
                for await (let packet of substream)
                {
                    receivedHelloPackets.push(packet);
                }
            })(),
            (async () =>
            {
                let substream = demux.stream('abc');
                for await (let packet of substream)
                {
                    receivedAbcPackets.push(packet);
                }
            })(),
        ]);

        expect(receivedHelloPackets.length).toBe(10);
        expect(receivedHelloPackets[0]).toBe('world0');
        expect(receivedHelloPackets[1]).toBe('world1');
        expect(receivedHelloPackets[9]).toBe('world9');

        expect(receivedAbcPackets.length).toBe(10);
        expect(receivedAbcPackets[0]).toBe('def0');
        expect(receivedAbcPackets[1]).toBe('def1');
        expect(receivedAbcPackets[9]).toBe('def9');
    });

    it('should support iterating over a single substream from multiple consumers at the same time', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                demux.write('hello', 'world' + i);
            }
            demux.close('hello');
        })();

        let receivedPacketsA = [];
        let receivedPacketsB = [];
        let receivedPacketsC = [];
        let substream        = demux.stream('hello');

        await Promise.all([
            (async () =>
            {
                for await (let packet of substream)
                {
                    receivedPacketsA.push(packet);
                }
            })(),
            (async () =>
            {
                for await (let packet of substream)
                {
                    receivedPacketsB.push(packet);
                }
            })(),
            (async () =>
            {
                for await (let packet of substream)
                {
                    receivedPacketsC.push(packet);
                }
            })(),
        ]);

        expect(receivedPacketsA.length).toBe(10);
        expect(receivedPacketsB.length).toBe(10);
        expect(receivedPacketsC.length).toBe(10);
    });

    it('should support iterating over a substream using a while loop', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                demux.write('hello', 'world' + i);
                demux.write('hello', 'foo' + i);
            }
            demux.close('hello');
        })();

        let receivedPackets = [];
        let asyncIterator   = demux.stream('hello').createConsumer();

        while (true)
        {
            let packet = await asyncIterator.next();
            if (packet.done) break;
            receivedPackets.push(packet.value);
        }

        expect(receivedPackets.length).toBe(20);
        expect(receivedPackets[0]).toBe('world0');
        expect(receivedPackets[1]).toBe('foo0');
        expect(receivedPackets[2]).toBe('world1');
        expect(receivedPackets[3]).toBe('foo1');
    });

    it('should support closing all streams using a single closeAll command', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                demux.write('hello', 'world' + i);
                demux.write('abc', 'def' + i);
            }
            demux.closeAll();
        })();

        let receivedHelloPackets = [];
        let receivedAbcPackets   = [];

        await Promise.all([
            (async () =>
            {
                let substream = demux.stream('hello');
                for await (let packet of substream)
                {
                    receivedHelloPackets.push(packet);
                }
            })(),
            (async () =>
            {
                let substream = demux.stream('abc');
                for await (let packet of substream)
                {
                    receivedAbcPackets.push(packet);
                }
            })(),
        ]);

        expect(receivedHelloPackets.length).toBe(10);
        expect(receivedAbcPackets.length).toBe(10);
    });

    it('should support resuming stream consumption after the stream has been closed using closeAll', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                demux.write('hello', 'a' + i);
            }
            demux.closeAll();
        })();

        let receivedPacketsA = [];
        for await (let packet of demux.stream('hello'))
        {
            receivedPacketsA.push(packet);
        }

        expect(receivedPacketsA.length).toBe(10);

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                demux.write('hello', 'b' + i);
            }
            demux.closeAll();
        })();

        let receivedPacketsB = [];
        for await (let packet of demux.stream('hello'))
        {
            receivedPacketsB.push(packet);
        }

        expect(receivedPacketsB.length).toBe(10);
    });

    it('should support the stream.once() method', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 10; i++)
            {
                await wait(10);
                demux.write('hello', 'world' + i);
            }
            demux.close('hello');
        })();

        let substream = demux.stream('hello');

        let packet = await substream.once();
        expect(packet).toBe('world0');

        packet = await substream.once();
        expect(packet).toBe('world1');

        packet = await substream.once();
        expect(packet).toBe('world2');
    });

    it('should not resolve stream.once() when stream is closed', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            await wait(10);
            demux.close('hello');
        })();

        let substream       = demux.stream('hello');
        let receivedPackets = [];

        (async () =>
        {
            let packet = await substream.once();
            receivedPackets.push(packet);
        })();

        await wait(100);
        expect(receivedPackets.length).toBe(0);
    });

    it('should support the stream.once() method with timeout', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 3; i++)
            {
                await wait(20);
                demux.write('hello', 'world' + i);
            }
            demux.close('hello');
        })();

        let substream = demux.stream('hello');

        let packet = await substream.once(30);
        expect(packet).toBe('world0');

        let error;
        packet = null;
        try
        {
            packet = await substream.once(10);
        }
        catch (err)
        {
            error = err;
        }
        expect(error).not.toBeUndefined();
        expect(error.name).toBe('TimeoutError');
        expect(packet).toBe(null);
    });

    it('should support stream.next() method with close command', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            for (let i = 0; i < 3; i++)
            {
                await wait(10);
                demux.write('hello', 'world' + i);
            }
            await wait(10);
            demux.close('hello');
        })();

        let substream = demux.stream('hello');

        let packet = await substream.next();
        expect(JSON.stringify(packet)).toBe(JSON.stringify({ value: 'world0', done: false }));

        packet = await substream.next();
        expect(JSON.stringify(packet)).toBe(JSON.stringify({ value: 'world1', done: false }));

        packet = await substream.next();
        expect(JSON.stringify(packet)).toBe(JSON.stringify({ value: 'world2', done: false }));

        packet = await substream.next();
        expect(JSON.stringify(packet)).toBe(JSON.stringify({ value: undefined, done: true }));
    });

    it('should support stream.next() method with closeAll command', async () =>
    {
        const demux = new StreamDemux();

        (async () =>
        {
            await wait(10);
            demux.write('hello', 'world');
            await wait(10);
            demux.closeAll();
        })();

        let substream = demux.stream('hello');

        let packet = await substream.next();
        expect(JSON.stringify(packet)).toBe(JSON.stringify({ value: 'world', done: false }));

        packet = await substream.next();
        expect(JSON.stringify(packet)).toBe(JSON.stringify({ value: undefined, done: true }));
    });
});
