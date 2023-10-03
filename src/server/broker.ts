import { SimpleBroker } from '@topgunbuild/socket/simple-broker';
import { listFilterMatch, storageListOptionsFromGetOptions } from '../storage';

export class TGBroker extends SimpleBroker
{
    publish(channelName: string, data: any, suppressEvent?: boolean): Promise<void>
    {
        const packet            = {
            channel: channelName,
            data
        };
        const subscriberSockets = this._clientSubscribers[channelName] || {};
        const subscriberOptions = this._clientSubscribersOptions[channelName] || {};

        Object.keys(subscriberSockets)
            .filter((socketId) =>
            {
                const options      = subscriberOptions[socketId];
                const soul         = channelName.replace(/^topgun\/nodes\//, '');
                const queryOptions = storageListOptionsFromGetOptions(options.data);

                return listFilterMatch(queryOptions, soul);
                // const match        = listFilterMatch(queryOptions, soul);
                // console.log({ soul, match });
                // return true;
            })
            .forEach((socketId) =>
            {
                subscriberSockets[socketId].transmit('#publish', packet);
            });

        if (!suppressEvent)
        {
            this.emit('publish', packet);
        }
        return Promise.resolve();
    }
}
