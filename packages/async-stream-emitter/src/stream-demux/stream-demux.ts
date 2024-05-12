import { StreamDemuxValue } from './stream-demux-value';
import { WritableConsumableStream } from '../writable-consumable-stream/writable-consumable-stream';
import { ConsumerStats } from '../writable-consumable-stream/consumer-stats';
import { Consumer } from '../writable-consumable-stream/consumer';
import { DemuxedConsumableStream } from './demuxed-consumable-stream';


export class StreamDemux<T>
{
    private _mainStream: WritableConsumableStream<StreamDemuxValue<T>|T>;

    /**
     * Constructor
     */
    constructor()
    {
        this._mainStream = new WritableConsumableStream<StreamDemuxValue<T>>();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    write(streamName: string, value: T): void
    {
        this._mainStream.write({
            stream: streamName,
            data  : {
                value,
                done: false,
            },
        });
    }

    close(streamName: string, value?: T): void
    {
        this._mainStream.write({
            stream: streamName,
            data  : {
                value,
                done: true,
            },
        });
    }

    closeAll(value?: T): void
    {
        this._mainStream.close(value);
    }

    writeToConsumer(consumerId: number, value: T): void
    {
        this._mainStream.writeToConsumer(consumerId, {
            consumerId,
            data: {
                value,
                done: false,
            },
        });
    }

    closeConsumer(consumerId: number, value: T): void
    {
        this._mainStream.closeConsumer(consumerId, {
            consumerId,
            data: {
                value,
                done: true,
            },
        });
    }

    getConsumerStats(consumerId: number): ConsumerStats
    {
        return this._mainStream.getConsumerStats(consumerId);
    }

    getConsumerStatsList(streamName: string): ConsumerStats[]
    {
        const consumerList = this._mainStream.getConsumerStatsList();
        return consumerList.filter((consumerStats) =>
        {
            return consumerStats.stream === streamName;
        });
    }

    getConsumerStatsListAll(): ConsumerStats[]
    {
        return this._mainStream.getConsumerStatsList();
    }

    kill(streamName: string, value?: T): void
    {
        const consumerList = this.getConsumerStatsList(streamName);
        const len          = consumerList.length;
        for (let i = 0; i < len; i++)
        {
            this.killConsumer(consumerList[i].id, value);
        }
    }

    killAll(value?: T): void
    {
        this._mainStream.kill(value);
    }

    killConsumer(consumerId: number, value?: T): void
    {
        this._mainStream.killConsumer(consumerId, value);
    }

    getBackpressure(streamName: string): number
    {
        const consumerList = this.getConsumerStatsList(streamName);
        const len          = consumerList.length;

        let maxBackpressure = 0;
        for (let i = 0; i < len; i++)
        {
            const consumer = consumerList[i];
            if (consumer.backpressure > maxBackpressure)
            {
                maxBackpressure = consumer.backpressure;
            }
        }
        return maxBackpressure;
    }

    getBackpressureAll(): number
    {
        return this._mainStream.getBackpressure();
    }

    getConsumerBackpressure(consumerId: number): number
    {
        return this._mainStream.getConsumerBackpressure(consumerId);
    }

    hasConsumer(streamName: string, consumerId: number): boolean
    {
        const consumerStats = this._mainStream.getConsumerStats(consumerId);
        return !!consumerStats && consumerStats.stream === streamName;
    }

    hasConsumerAll(consumerId: number): boolean
    {
        return this._mainStream.hasConsumer(consumerId);
    }

    getConsumerCount(streamName: string): number
    {
        return this.getConsumerStatsList(streamName).length;
    }

    getConsumerCountAll(): number
    {
        return this.getConsumerStatsListAll().length;
    }

    createConsumer(
        streamName: string,
        timeout: any
    ): Consumer<StreamDemuxValue<T>|T>
    {
        const mainStreamConsumer = this._mainStream.createConsumer(timeout);

        const consumerNext      = mainStreamConsumer.next;
        mainStreamConsumer.next = async function ()
        {
            while (true)
            {
                const argumentsTyped: any = arguments;
                const packet              = await consumerNext.apply(this, argumentsTyped);
                if (packet.value)
                {
                    if (
                        packet.value.stream === streamName ||
                        packet.value.consumerId === this.id
                    )
                    {
                        if (packet.value.data.done)
                        {
                            this.return();
                        }
                        return packet.value.data;
                    }
                }
                if (packet.done)
                {
                    return packet;
                }
            }
        };

        const consumerGetStats      = mainStreamConsumer.getStats;
        mainStreamConsumer.getStats = function ()
        {
            const argumentsTyped: any = arguments;
            const stats               = consumerGetStats.apply(this, argumentsTyped);
            stats.stream              = streamName;
            return stats;
        };

        const consumerApplyBackpressure      = mainStreamConsumer.applyBackpressure;
        mainStreamConsumer.applyBackpressure = function (packet)
        {
            const argumentsTyped: any = arguments;

            if (packet.value)
            {
                if (
                    packet.value.stream === streamName ||
                    packet.value.consumerId === this.id
                )
                {
                    consumerApplyBackpressure.apply(this, argumentsTyped);

                    return;
                }
            }
            if (packet.done)
            {
                consumerApplyBackpressure.apply(this, argumentsTyped);
            }
        };

        const consumerReleaseBackpressure      = mainStreamConsumer.releaseBackpressure;
        mainStreamConsumer.releaseBackpressure = function (packet)
        {
            const argumentsTyped: any = arguments;

            if (packet.value)
            {
                if (
                    packet.value.stream === streamName ||
                    packet.value.consumerId === this.id
                )
                {
                    consumerReleaseBackpressure.apply(this, argumentsTyped);

                    return;
                }
            }
            if (packet.done)
            {
                consumerReleaseBackpressure.apply(this, argumentsTyped);
            }
        };

        return mainStreamConsumer;
    }

    stream(streamName: string): DemuxedConsumableStream<T>
    {
        return new DemuxedConsumableStream(this, streamName);
    }
}
