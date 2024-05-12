import {
    ConsumableStream,
    ConsumableStreamConsumer,
    DemuxedConsumableStream,
    StreamDemux
} from '@topgunbuild/async-stream-emitter';
import { IClientSocket } from '../client/types';
import { ChannelState, ChannelOptions } from './types';

export class Channel<T> extends ConsumableStream<T>
{
    static PENDING: ChannelState      = 'pending';
    static SUBSCRIBED: ChannelState   = 'subscribed';
    static UNSUBSCRIBED: ChannelState = 'unsubscribed';

    readonly PENDING: ChannelState;
    readonly SUBSCRIBED: ChannelState;
    readonly UNSUBSCRIBED: ChannelState;

    name: string;
    client: IClientSocket;
    _pendingSubscriptionCid: number;

    private _eventDemux: StreamDemux<T>;
    private _dataStream: DemuxedConsumableStream<T>;

    /**
     * Constructor
     */
    constructor(name: string, client: IClientSocket, eventDemux: StreamDemux<T>, dataStream: DemuxedConsumableStream<T>)
    {
        super();
        this.PENDING      = Channel.PENDING;
        this.SUBSCRIBED   = Channel.SUBSCRIBED;
        this.UNSUBSCRIBED = Channel.UNSUBSCRIBED;

        this.name   = name;
        this.client = client;

        this._eventDemux = eventDemux;
        this._dataStream = dataStream;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Accessors
    // -----------------------------------------------------------------------------------------------------

    get state(): ChannelState
    {
        return this.client.getChannelState(this.name);
    }

    set state(value: ChannelState)
    {
        throw new Error('Cannot directly set channel state');
    }

    get options(): ChannelOptions
    {
        return this.client.getChannelOptions(this.name);
    }

    set options(value: ChannelOptions)
    {
        throw new Error('Cannot directly set channel options');
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    createConsumer(timeout?: number): ConsumableStreamConsumer<T>
    {
        return this._dataStream.createConsumer(timeout);
    }

    listener(eventName: string): DemuxedConsumableStream<T>
    {
        return this._eventDemux.stream(`${this.name}/${eventName}`);
    }

    closeListener(eventName: string): void
    {
        this._eventDemux.close(`${this.name}/${eventName}`);
    }

    closeAllListeners(): void
    {
        this._eventDemux.closeAll();
    }

    close(): void
    {
        this.client.closeChannel(this.name);
    }

    subscribe(options: ChannelOptions): void
    {
        this.client.subscribe(this.name, options);
    }

    unsubscribe(): void
    {
        this.client.unsubscribe(this.name);
    }

    isSubscribed(includePending?: boolean): boolean
    {
        return this.client.isSubscribed(this.name, includePending);
    }

    publish(data: any): void
    {
        return this.client.publish(this.name, data);
    }
}
