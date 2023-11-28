import {
    ConsumableStream,
    DemuxedConsumableStream,
    StreamDemux,
    ConsumableStreamConsumer
} from '@topgunbuild/async-stream-emitter';
import { isObject, isString, isNumber, cloneValue } from '@topgunbuild/typed';
import { uuidv4 } from '../utils/uuidv4';
import { TGStreamState } from './types';
import { TGExchange } from './exchange';
import { TGCollectionChangeEvent, TGCollectionOptions, TGData, TGNode } from '../types';
import { getNodeSoul, isNode } from '../utils';
import { diffCRDT } from '../crdt';
import { debounce } from '../utils/debounce';

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

    private _eventDemux: StreamDemux<any>;
    private _dataStream: DemuxedConsumableStream<T>;

    nodes: TGNode[];
    lastNode: TGNode;
    existingNodesMap: Record<string, TGNode>;

    /**
     * Constructor
     */
    constructor(
        name: string = uuidv4(),
        exchange: TGExchange,
        eventDemux: StreamDemux<any>,
        dataStream: DemuxedConsumableStream<T>,
        attributes: {[key: string]: any}
    )
    {
        super();
        this.name             = name;
        this.PENDING          = TGStream.PENDING;
        this.SUBSCRIBED       = TGStream.SUBSCRIBED;
        this.UNSUBSCRIBED     = TGStream.UNSUBSCRIBED;
        this.DESTROYED        = TGStream.DESTROYED;
        this.exchange         = exchange;
        this._eventDemux      = eventDemux;
        this._dataStream      = dataStream;
        this.attributes       = attributes;
        this.existingNodesMap = {};
        this.nodes            = [];
        this.lastNode         = null;

        if (isObject(this.attributes['topGunCollection']))
        {
            const collectionOptions = this.attributes['topGunCollection'] as TGCollectionOptions;

            const eventPublish = event => this._eventDemux.write(`${this.name}/collectionChange`, event);
            const publish      = isNumber(collectionOptions.debounce)
                ? debounce(eventPublish, collectionOptions.debounce)
                : eventPublish;

            (async () =>
            {
                for await (const { key, value } of this._dataStream as DemuxedConsumableStream<TGData<TGNode>>)
                {
                    // Detect changes
                    const emptyChange    = !value && (this.nodes.length === 0 || !this.existingNodesMap[key]);
                    const updatedGraph   = { [key]: value };
                    const existingGraph  = { [key]: this.existingNodesMap[key] };
                    const valueIsNode    = isNode(value);
                    const exists         = isNode(this.existingNodesMap[key]);
                    const nodeNotChanged = exists && valueIsNode && !diffCRDT(updatedGraph, existingGraph);

                    // Abort if data has not changed
                    if (emptyChange || nodeNotChanged)
                    {
                        continue;
                    }

                    const oldValue = [...this.nodes];

                    if (valueIsNode)
                    {
                        const node = cloneValue(value);

                        if (isString(collectionOptions?.idField))
                        {
                            node[collectionOptions.idField] = getNodeSoul(node).split('/').pop();
                        }
                        if (isString(collectionOptions?.keyField))
                        {
                            node[collectionOptions.keyField] = key;
                        }

                        if (exists)
                        {
                            const index       = this.#getNodeIndex(key);
                            this.nodes[index] = node;
                        }
                        else
                        {
                            this.nodes.push(node);
                        }
                        this.existingNodesMap[key] = node;
                    }
                    else if (exists)
                    {
                        const index = this.#getNodeIndex(key);
                        this.nodes.splice(index, 1);
                        delete this.existingNodesMap[key];
                    }

                    const event: TGCollectionChangeEvent = {
                        oldValue,
                        newValue: [...this.nodes],
                        nodes   : this.nodes
                    };

                    publish(event);
                }
            })();
        }
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

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #getNodeIndex(soul: string)
    {
        return this.nodes.findIndex(node => getNodeSoul(node) === getNodeSoul(this.existingNodesMap[soul]));
    }
}