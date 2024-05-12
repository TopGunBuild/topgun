import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { SimpleExchange } from './simple-exchange';
import { SimpleSocket } from './types';
import { ServerChannelOptions } from '../server/types';

export class SimpleBroker extends AsyncStreamEmitter<any>
{
    isReady: boolean;
    protected _exchangeClient: SimpleExchange;
    protected _clientSubscribers: {
        [channelName: string]: {
            [socketId: string]: SimpleSocket
        }
    };
    protected _clientSubscribersOptions: {
        [channelName: string]: {
            [socketId: string]: ServerChannelOptions
        }
    };
    protected _clientSubscribersCounter: {
        [channelName: string]: number
    };

    /**
     * constructor
     */
    constructor()
    {
        super();
        this.isReady                   = false;
        this._exchangeClient           = new SimpleExchange(this);
        this._clientSubscribers        = {};
        this._clientSubscribersOptions = {};
        this._clientSubscribersCounter = {};

        setTimeout(() =>
        {
            this.isReady = true;
            this.emit('ready', {});
        }, 0);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    exchange(): SimpleExchange
    {
        return this._exchangeClient;
    }

    subscribeSocket(socket: SimpleSocket, channelOptions: ServerChannelOptions): Promise<any>
    {
        const channelName = channelOptions.channel;

        if (!this._clientSubscribers[channelName])
        {
            this._clientSubscribers[channelName]        = {};
            this._clientSubscribersOptions[channelName] = {};
            this._clientSubscribersCounter[channelName] = 0;
        }
        if (!this._clientSubscribers[channelName][socket.id])
        {
            this._clientSubscribersCounter[channelName]++;
            this.emit('subscribe', {
                channel: channelName,
            });
        }
        this._clientSubscribers[channelName][socket.id]        = socket;
        this._clientSubscribersOptions[channelName][socket.id] = channelOptions;
        return Promise.resolve();
    }

    unsubscribeSocket(socket: SimpleSocket, channelName: string): Promise<any>
    {
        if (this._clientSubscribers[channelName])
        {
            if (this._clientSubscribers[channelName][socket.id])
            {
                this._clientSubscribersCounter[channelName]--;
                delete this._clientSubscribers[channelName][socket.id];
                delete this._clientSubscribersOptions[channelName][socket.id];

                if (this._clientSubscribersCounter[channelName] <= 0)
                {
                    delete this._clientSubscribers[channelName];
                    delete this._clientSubscribersCounter[channelName];
                    this.emit('unsubscribe', {
                        channel: channelName,
                    });
                }
            }
        }
        return Promise.resolve();
    }

    subscriptions(): string[]
    {
        return Object.keys(this._clientSubscribers);
    }

    isSubscribed(channelName: string): boolean
    {
        return !!this._clientSubscribers[channelName];
    }

    publish(channelName: string, data: any, suppressEvent?: boolean): Promise<void>
    {
        const packet            = {
            channel: channelName,
            data,
        };
        const subscriberSockets = this._clientSubscribers[channelName] || {};

        Object.keys(subscriberSockets).forEach((i) =>
        {
            subscriberSockets[i].transmit('#publish', packet);
        });

        if (!suppressEvent)
        {
            this.emit('publish', packet);
        }
        return Promise.resolve();
    }
}
