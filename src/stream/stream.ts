import {
    ConsumableStream,
    DemuxedConsumableStream,
    StreamDemux,
    ConsumableStreamConsumer
} from '@topgunbuild/async-stream-emitter';
import { uuidv4 } from '../utils/uuidv4';
import { TGStreamState } from './types';
import { TGExchange } from './exchange';

export class TGStream<T> extends ConsumableStream<T>
{
    static PENDING: TGStreamState      = 'pending';
    static SUBSCRIBED: TGStreamState   = 'subscribed';
    static UNSUBSCRIBED: TGStreamState = 'unsubscribed';
    static DESTROYED: TGStreamState    = 'destroyed';

    readonly PENDING: TGStreamState;
    readonly SUBSCRIBED: TGStreamState;
    readonly UNSUBSCRIBED: TGStreamState;
    readonly DESTROYED: TGStreamState;

    readonly name: string;
    readonly exchange: TGExchange;
    readonly attributes: {[key: string]: any};

    private _eventDemux: StreamDemux<T>;
    private _dataStream: DemuxedConsumableStream<T>;

    /**
     * Constructor
     */
    constructor(
        name: string = uuidv4(),
        exchange: TGExchange,
        eventDemux: StreamDemux<T>,
        dataStream: DemuxedConsumableStream<T>,
        attributes: {[key: string]: any}
    )
    {
        super();
        this.name         = name;
        this.PENDING      = TGStream.PENDING;
        this.SUBSCRIBED   = TGStream.SUBSCRIBED;
        this.UNSUBSCRIBED = TGStream.UNSUBSCRIBED;
        this.DESTROYED    = TGStream.DESTROYED;
        this.exchange     = exchange;
        this._eventDemux  = eventDemux;
        this._dataStream  = dataStream;
        this.attributes   = attributes;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Accessors
    // -----------------------------------------------------------------------------------------------------

    get state(): TGStreamState
    {
        return this.exchange.getStreamState(this.name);
    }

    set state(value: TGStreamState)
    {
        throw new Error('Cannot directly set channel state');
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

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

    listener(eventName: string): DemuxedConsumableStream<T>
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
}