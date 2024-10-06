import { isObject, randomId } from '@topgunbuild/utils';
import { Exchange } from './exchange';
import { DataStreamState } from './types';
import { ConsumableStream, ConsumableStreamConsumer } from '../consumable-stream';
import { DemuxedConsumableStream, StreamDemux } from '../stream-demux';

export class DataStream<T> extends ConsumableStream<T>
{
    static PENDING: DataStreamState      = 'pending';
    static SUBSCRIBED: DataStreamState   = 'subscribed';
    static UNSUBSCRIBED: DataStreamState = 'unsubscribed';
    static DESTROYED: DataStreamState    = 'destroyed';

    readonly PENDING: DataStreamState;
    readonly SUBSCRIBED: DataStreamState;
    readonly UNSUBSCRIBED: DataStreamState;
    readonly DESTROYED: DataStreamState;

    readonly name: string;
    readonly exchange: Exchange;
    readonly attributes: {[key: string]: any};

    private _eventDemux: StreamDemux<any>;
    private _dataStream: DemuxedConsumableStream<T>;

    nodes: T[];
    lastNode: T;
    nodeMap: Record<string, T>;

    constructor(
        name: string = randomId(),
        exchange: Exchange,
        eventDemux: StreamDemux<any>,
        dataStream: DemuxedConsumableStream<T>,
        attributes: {[key: string]: any}
    )
    {
        super();
        this.name             = name;
        this.PENDING          = DataStream.PENDING;
        this.SUBSCRIBED       = DataStream.SUBSCRIBED;
        this.UNSUBSCRIBED     = DataStream.UNSUBSCRIBED;
        this.DESTROYED        = DataStream.DESTROYED;
        this.exchange         = exchange;
        this._eventDemux      = eventDemux;
        this._dataStream      = dataStream;
        this.attributes = attributes;
        this.nodeMap    = {};
        this.nodes      = [];
        this.lastNode         = null;

        if (isObject(this.attributes['topGunCollection']))
        {
            // TODO: Update nodes
        }
    }

    get state(): DataStreamState
    {
        return this.exchange.getStreamState(this.name);
    }

    set state(value: DataStreamState)
    {
        throw new Error('Cannot directly set channel state');
    }

    createConsumer(timeout?: number): ConsumableStreamConsumer<T>
    {
        return this._dataStream.createConsumer(timeout);
    }

    subscribe(): void
    {
        this.exchange.subscribe(this.name);
    }

    publish(data: T): Promise<void>
    {
        return this.exchange.publish(this.name, data);
    }

    unsubscribe(): void
    {
        this.exchange.unsubscribe(this.name);
    }

    destroy(): void
    {
        this.exchange.destroy(this.name);
    }

    listener<E>(eventName: string): DemuxedConsumableStream<E>
    {
        return this._eventDemux.stream(`${this.name}/${eventName}`);
    }

    closeListener(eventName: string): void
    {
        this._eventDemux.close(`${this.name}/${eventName}`);
    }

    closeAllListeners(): void
    {
        this._eventDemux.closeAll();
    }

    // #getNodeIndex(soul: string)
    // {
    //     return this.nodes.findIndex(node => getNodeSoul(node) === getNodeSoul(this.existingNodesMap[soul]));
    // }
}
