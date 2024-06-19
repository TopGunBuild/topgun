import { AsyncStreamEmitter, StreamDemux } from '@topgunbuild/async-stream-emitter';
import { randomId } from '@topgunbuild/utils';
import { SimpleDataStream, DataStreamState } from './types';
import { DataStream } from './data-stream';

export class Exchange extends AsyncStreamEmitter<any>
{
    private readonly _streamDataDemux: StreamDemux<any>;
    private readonly _streamEventDemux: StreamDemux<any>;
    private readonly _streamMap: {
        [key: string]: SimpleDataStream
    };

    /**
     * Constructor
     */
    constructor()
    {
        super();
        this._streamMap        = {};
        this._streamEventDemux = new StreamDemux();
        this._streamDataDemux  = new StreamDemux();

        (async () =>
        {
            for await (const { streamName, data } of this.listener('publish'))
            {
                if (this._streamMap[streamName])
                {
                    this._streamDataDemux.write(streamName, data);
                }
            }
        })();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    publish(streamName: string, data: any): Promise<void>
    {
        this.emit('publish', { streamName, data });
        return Promise.resolve();
    }

    stream<T>(streamName: string, attributes: {[key: string]: any} = {}): DataStream<T>
    {
        const channelDataStream = this._streamDataDemux.stream(streamName);

        return new DataStream(
            streamName,
            this,
            this._streamEventDemux,
            channelDataStream,
            attributes
        );
    }

    destroy(streamName?: string): void
    {
        if (streamName)
        {
            const stream = this._streamMap[streamName];

            if (stream)
            {
                this.#triggerStreamDestroy(stream);
            }

            this._streamDataDemux.close(streamName);
        }
        else
        {
            Object.keys(this._streamMap).forEach((streamName) =>
            {
                const stream = this._streamMap[streamName];
                this.#triggerStreamDestroy(stream);
            });
            this.closeAllListeners();
            this._streamDataDemux.closeAll();
            this._streamEventDemux.closeAll();
        }
    }

    getStreamState(streamName: string): DataStreamState
    {
        const channel = this._streamMap[streamName];
        if (channel)
        {
            return channel.state;
        }
        return DataStream.UNSUBSCRIBED;
    }

    subscribe<T>(streamName: string = randomId(), attributes: {[key: string]: any} = {}): DataStream<T>
    {
        let stream = this._streamMap[streamName];

        if (!stream)
        {
            stream                     = {
                name : streamName,
                state: DataStream.PENDING,
                attributes
            };
            this._streamMap[streamName] = stream;
            this.#triggerStreamSubscribe(stream);
        }

        const channelDataStream = this._streamDataDemux.stream(streamName);

        return new DataStream(
            streamName,
            this,
            this._streamEventDemux,
            channelDataStream,
            attributes
        );
    }

    unsubscribe(streamName: string): void
    {
        const channel = this._streamMap[streamName];

        if (channel)
        {
            this.#triggerStreamUnsubscribe(channel);
        }
    }

    subscriptions(includePending?: boolean): string[]
    {
        const subs = [];
        Object.keys(this._streamMap).forEach((streamName) =>
        {
            if (includePending || this._streamMap[streamName].state === DataStream.SUBSCRIBED)
            {
                subs.push(streamName);
            }
        });
        return subs;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #triggerStreamDestroy(stream: SimpleDataStream): void
    {
        const streamName = stream.name;

        stream.state = DataStream.DESTROYED;

        delete this._streamMap[streamName];
        this._streamEventDemux.write(`${streamName}/destroy`, {});
        this.emit('destroy', { streamName });
    }

    #triggerStreamSubscribe(stream: SimpleDataStream): void
    {
        const streamName = stream.name;

        stream.state = DataStream.SUBSCRIBED;

        this._streamEventDemux.write(`${streamName}/subscribe`, {});
        this.emit('subscribe', { streamName });
    }

    #triggerStreamUnsubscribe(stream: SimpleDataStream): void
    {
        const streamName = stream.name;

        delete this._streamMap[streamName];
        if (stream.state === DataStream.SUBSCRIBED)
        {
            this._streamEventDemux.write(`${streamName}/unsubscribe`, {});
            this.emit('unsubscribe', { streamName });
        }
    }
}
