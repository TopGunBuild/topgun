import { SimpleBroker, PublishData } from '@topgunbuild/socket/simple-broker';
import { listFilterMatch, queryOptionsFromGetOptions } from '../storage';

export class TGBroker extends SimpleBroker
{
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
            this.handleLexQueries(channelName, data);
        }

        if (!suppressEvent)
        {
            this.emit('publish', packet);
        }
        return Promise.resolve();
    }

    handleLexQueries(originalChannelName: string, data: any): void
    {
        // Get node soul
        const soul = originalChannelName.replace(/^topgun\/nodes\//, '');

        // Get 'topgun/nodes/{soul}' from original channel name
        const channelName = originalChannelName.split('/').slice(0, 3).join('/');

        const packet: PublishData = {
            channel: channelName,
            data
        };
        const subscriberSockets   = this._clientSubscribers[channelName] || {};
        const subscriberOptions   = this._clientSubscribersOptions[channelName] || {};

        Object.keys(subscriberSockets)
            .filter((socketId) =>
            {
                const queryOptions = queryOptionsFromGetOptions(subscriberOptions[socketId]?.data);
                return listFilterMatch(queryOptions, soul);
            })
            .forEach((socketId) =>
            {
                subscriberSockets[socketId].transmit('#publish', packet);
            });
    }
}
