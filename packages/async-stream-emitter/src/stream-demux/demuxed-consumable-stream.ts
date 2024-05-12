import { ConsumableStream, ConsumableStreamConsumer } from '../consumable-stream/consumable-stream';
import { ConsumerStats } from '../writable-consumable-stream';
import { StreamDemux } from './stream-demux';

export class DemuxedConsumableStream<T> extends ConsumableStream<T>
{
    name: string;
    private _streamDemux: StreamDemux<any>;

    /**
     * Constructor
     */
    constructor(streamDemux: any, name: string)
    {
        super();
        this.name         = name;
        this._streamDemux = streamDemux;
    }

    createConsumer(timeout?: number): ConsumableStreamConsumer<T>
    {
        return this._streamDemux.createConsumer(this.name, timeout);
    }

    hasConsumer(consumerId: number): boolean
    {
        return this._streamDemux.hasConsumer(this.name, consumerId);
    }

    getConsumerStats(consumerId: number): ConsumerStats
    {
        if (!this.hasConsumer(consumerId))
        {
            return undefined;
        }
        return this._streamDemux.getConsumerStats(consumerId);
    }

    getConsumerStatsList(): ConsumerStats[]
    {
        return this._streamDemux.getConsumerStatsList(this.name);
    }

    getBackpressure(): number
    {
        return this._streamDemux.getBackpressure(this.name);
    }

    getConsumerBackpressure(consumerId: number): number
    {
        if (!this.hasConsumer(consumerId))
        {
            return 0;
        }
        return this._streamDemux.getConsumerBackpressure(consumerId);
    }
}
