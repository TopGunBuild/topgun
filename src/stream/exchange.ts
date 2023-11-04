import { AsyncStreamEmitter, StreamDemux } from '@topgunbuild/async-stream-emitter';
import { TGStream } from './stream';
import { TGSimpleStream, TGStreamState } from './types';
import { uuidv4 } from '../utils/uuidv4';

export class TGExchange extends AsyncStreamEmitter<any>
{
    private readonly _streamDataDemux: StreamDemux<any>;
    private readonly _streamEventDemux: StreamDemux<any>;
    private readonly _streamMap: {
        [key: string]: TGSimpleStream
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

    stream<T>(streamName: string, attributes: {[key: string]: any} = {}): TGStream<T>
    {
        const channelDataStream = this._streamDataDemux.stream(streamName);

        return new TGStream(
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

    getStreamState(streamName: string): TGStreamState
    {
        const channel = this._streamMap[streamName];
        if (channel)
        {
            return channel.state;
        }
        return TGStream.UNSUBSCRIBED;
    }

    subscribe<T>(streamName: string = uuidv4(), attributes: {[key: string]: any} = {}): TGStream<T>
    {
        let channel = this._streamMap[streamName];

        if (!channel)
        {
            channel                     = {
                name : streamName,
                state: TGStream.PENDING,
                attributes
            };
            this._streamMap[streamName] = channel;
            this.#triggerStreamSubscribe(channel);
        }

        const channelDataStream = this._streamDataDemux.stream(streamName);

        return new TGStream(
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
            if (includePending || this._streamMap[streamName].state === TGStream.SUBSCRIBED)
            {
                subs.push(streamName);
            }
        });
        return subs;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #triggerStreamDestroy(stream: TGSimpleStream): void
    {
        const streamName = stream.name;

        stream.state = TGStream.DESTROYED;

        delete this._streamMap[streamName];
        this._streamEventDemux.write(`${streamName}/destroy`, {});
        this.emit('destroy', { streamName });
    }

    #triggerStreamSubscribe(stream: TGSimpleStream): void
    {
        const streamName = stream.name;

        stream.state = TGStream.SUBSCRIBED;

        this._streamEventDemux.write(`${streamName}/subscribe`, {});
        this.emit('subscribe', { streamName });
    }

    #triggerStreamUnsubscribe(stream: TGSimpleStream): void
    {
        const streamName = stream.name;

        delete this._streamMap[streamName];
        if (stream.state === TGStream.SUBSCRIBED)
        {
            this._streamEventDemux.write(`${streamName}/unsubscribe`, {});
            this.emit('unsubscribe', { streamName });
        }
    }
}
