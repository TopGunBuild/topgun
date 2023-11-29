import { SimpleBroker, PublishData } from '@topgunbuild/socket/simple-broker';
import { filterMatch } from '../storage';

export class TGBroker extends SimpleBroker
{
    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    publish(channelName: string, data: any, suppressEvent?: boolean): Promise<void>
    {
        const packet: PublishData = {
            channel: channelName,
            data
        };

        // Send to all subscribers to this soul
        const subscriberSockets = this._clientSubscribers[channelName] || {};
        Object.keys(subscriberSockets).forEach((socketId) =>
        {
            subscriberSockets[socketId].transmit('#publish', packet);
        });

        if (channelName.startsWith('topgun/nodes/'))
        {
            this.publishToCollections(channelName, data);
        }

        if (!suppressEvent)
        {
            this.emit('publish', packet);
        }
        return Promise.resolve();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private publishToCollections(channel: string, data: any): void
    {
        // Get node soul
        const soul = channel.replace(/^topgun\/nodes\//, '');

        // Get 'topgun/nodes/{soul}' from original channel name for collection queries
        const collectionChannel = channel
            .split('/')
            .slice(0, -1)
            .join('/');

        const packet = {
            channel: collectionChannel,
            data
        };

        const subscriberSockets = this._clientSubscribers[collectionChannel] || {};
        const subscriberOptions = this._clientSubscribersOptions[collectionChannel] || {};

        Object.keys(subscriberSockets)
            .filter((socketId) =>
            {
                const queryOptions = subscriberOptions[socketId]?.data;
                return filterMatch(soul, queryOptions);
            })
            .forEach((socketId) =>
            {
                subscriberSockets[socketId].transmit('#publish', packet);
            });
    }
}
