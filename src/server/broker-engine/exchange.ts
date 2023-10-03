import { SimpleBroker, SimpleExchange } from '@topgunbuild/socket/simple-broker';

export class TGExchange extends SimpleExchange
{
    constructor(broker: SimpleBroker)
    {
        super(broker);
    }

    publish(channelName: string, data: any): Promise<void>
    {
        return this._broker.publish(channelName, data);
    }
}
