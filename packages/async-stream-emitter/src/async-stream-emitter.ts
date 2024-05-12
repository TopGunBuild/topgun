import { DemuxedConsumableStream, StreamDemux } from './stream-demux';
import { ConsumerStats } from './writable-consumable-stream/consumer-stats';

export class AsyncStreamEmitter<T>
{
    private _listenerDemux: StreamDemux<T>;

    constructor()
    {
        this._listenerDemux = new StreamDemux();
    }

    emit(eventName: string, data: T): void
    {
        this._listenerDemux.write(eventName, data);
    }

    listener(eventName: string): DemuxedConsumableStream<T>
    {
        return this._listenerDemux.stream(eventName);
    }

    closeListener(eventName: string): void
    {
        this._listenerDemux.close(eventName);
    }

    closeAllListeners(): void
    {
        this._listenerDemux.closeAll();
    }

    getListenerConsumerStats(consumerId: number): ConsumerStats
    {
        return this._listenerDemux.getConsumerStats(consumerId);
    }

    getListenerConsumerStatsList(eventName: string): ConsumerStats[]
    {
        return this._listenerDemux.getConsumerStatsList(eventName);
    }

    getAllListenersConsumerStatsList(): ConsumerStats[]
    {
        return this._listenerDemux.getConsumerStatsListAll();
    }

    getListenerConsumerCount(eventName: string): number
    {
        return this._listenerDemux.getConsumerCount(eventName);
    }

    getAllListenersConsumerCount(): number
    {
        return this._listenerDemux.getConsumerCountAll();
    }

    killListener(eventName: string): void
    {
        this._listenerDemux.kill(eventName);
    }

    killAllListeners(): void
    {
        this._listenerDemux.killAll();
    }

    killListenerConsumer(consumerId: number): void
    {
        this._listenerDemux.killConsumer(consumerId);
    }

    getListenerBackpressure(eventName: string): number
    {
        return this._listenerDemux.getBackpressure(eventName);
    }

    getAllListenersBackpressure(): number
    {
        return this._listenerDemux.getBackpressureAll();
    }

    getListenerConsumerBackpressure(consumerId: number): number
    {
        return this._listenerDemux.getConsumerBackpressure(consumerId);
    }

    hasListenerConsumer(eventName: string, consumerId: number): boolean
    {
        return this._listenerDemux.hasConsumer(eventName, consumerId);
    }

    hasAnyListenerConsumer(consumerId: number): boolean
    {
        return this._listenerDemux.hasConsumerAll(consumerId);
    }
}
