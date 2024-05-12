import { AsyncStreamEmitter, StreamDemux } from '@topgunbuild/async-stream-emitter';
import { SimpleChannel } from './types';
import { Channel } from '../channel/channel';
import { SimpleBroker } from './simple-broker';
import { ChannelState } from '../channel/types';

/* eslint-disable @typescript-eslint/no-unused-vars */
export class SimpleExchange extends AsyncStreamEmitter<any>
{
    protected _broker: SimpleBroker;
    protected _channelMap: {
        [key: string]: SimpleChannel
    };
    protected _channelEventDemux: StreamDemux<any>;
    protected _channelDataDemux: StreamDemux<any>;

    /**
     * Constructor
     */
    constructor(broker: SimpleBroker)
    {
        super();

        this._broker            = broker;
        this._channelMap        = {};
        this._channelEventDemux = new StreamDemux();
        this._channelDataDemux  = new StreamDemux();

        (async () =>
        {
            for await (const { channel, data } of this._broker.listener('publish'))
            {
                this._channelDataDemux.write(channel, data);
            }
        })();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    destroy(): void
    {
        this._broker.closeAllListeners();
    }

    publish(channelName: string, data: any): Promise<void>
    {
        return this._broker.publish(channelName, data);
    }

    subscribe(channelName: string): Channel<any>
    {
        let channel = this._channelMap[channelName];

        if (!channel)
        {
            channel                       = {
                name : channelName,
                state: Channel.PENDING,
            };
            this._channelMap[channelName] = channel;
            this._triggerChannelSubscribe(channel);
        }

        const channelDataStream = this._channelDataDemux.stream(channelName);
        return new Channel(
            channelName,
            this,
            this._channelEventDemux,
            channelDataStream,
        );
    }

    unsubscribe(channelName: string): void
    {
        const channel = this._channelMap[channelName];

        if (channel)
        {
            this._triggerChannelUnsubscribe(channel);
        }
    }

    channel(channelName: string): Channel<any>
    {
        const channelDataStream = this._channelDataDemux.stream(channelName);
        const channelIterable   = new Channel(
            channelName,
            this,
            this._channelEventDemux,
            channelDataStream,
        );

        return channelIterable;
    }

    getChannelState(channelName: string): ChannelState
    {
        const channel = this._channelMap[channelName];
        if (channel)
        {
            return channel.state;
        }
        return Channel.UNSUBSCRIBED;
    }

    getChannelOptions(channelName: string)
    {
        return {};
    }

    subscriptions(includePending?: boolean): string[]
    {
        const subs = [];
        Object.keys(this._channelMap).forEach((channelName) =>
        {
            if (includePending || this._channelMap[channelName].state === Channel.SUBSCRIBED)
            {
                subs.push(channelName);
            }
        });
        return subs;
    }

    isSubscribed(channelName: string, includePending?: boolean): boolean
    {
        const channel = this._channelMap[channelName];
        if (includePending)
        {
            return !!channel;
        }
        return !!channel && channel.state === Channel.SUBSCRIBED;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    protected _triggerChannelSubscribe(channel: SimpleChannel): void
    {
        const channelName = channel.name;

        channel.state = Channel.SUBSCRIBED;

        this._channelEventDemux.write(`${channelName}/subscribe`, {});
        this.emit('subscribe', { channel: channelName });
    }

    protected _triggerChannelUnsubscribe(channel: SimpleChannel)
    {
        const channelName = channel.name;

        delete this._channelMap[channelName];
        if (channel.state === Channel.SUBSCRIBED)
        {
            this._channelEventDemux.write(`${channelName}/unsubscribe`, {});
            this.emit('unsubscribe', { channel: channelName });
        }
    }
}
