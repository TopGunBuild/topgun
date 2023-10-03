import { SimpleBroker, SimpleExchange } from '@topgunbuild/socket/simple-broker';

export class TGBroker extends SimpleBroker
{
    /**
     * Constructor
     */
    constructor()
    {
        super();
        this._exchangeClient           = new SimpleExchange(this);
    }

    publish(channelName: string, data: any, suppressEvent?: boolean): Promise<void>
    {
        const packet            = {
            channel: channelName,
            data
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
