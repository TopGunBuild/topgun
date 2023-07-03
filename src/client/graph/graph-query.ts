import { isFunction } from 'topgun-typed';
import { TGGet, TGGraphData, TGMessage, TGNode, TGOnCb, TGOptionsGet } from '../../types';
import { TGGraph } from './graph';
import { getNodeSoul } from '../../utils/node';
import { StorageListOptions } from '../../storage';
import { listFilterMatch, storageListOptionsFromGetOptions } from '../../storage/utils';
import { uuidv4 } from '../../utils/uuidv4';
import { TGExchange } from '../../stream/exchange';
import { TGStream } from '../../stream/stream';

export class TGGraphQuery extends TGExchange
{
    readonly queryString: string;
    readonly options: TGOptionsGet;

    private _endCurQuery?: () => void;
    private readonly _listOptions: StorageListOptions|null;
    private readonly _graph: TGGraph;
    private readonly _updateGraph: (
        data: TGGraphData,
        replyToId?: string,
    ) => void;

    /**
     * Constructor
     */
    constructor(
        graph: TGGraph,
        queryString: string,
        updateGraph: (data: TGGraphData, replyToId?: string) => void,
    )
    {
        super();
        this._onDirectQueryReply = this._onDirectQueryReply.bind(this);
        this.options             = JSON.parse(queryString);
        this.queryString         = queryString;
        this._graph              = graph;
        this._updateGraph        = updateGraph;
        this._listOptions        = storageListOptionsFromGetOptions(this.options)
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    listenerCount(): number
    {
        return this.subscriptions(true).length;
    }

    // get(cb?: TGOnCb<any>, msgId?: string): TGGraphQuery
    // {
    //     if (cb)
    //     {
    //         this.on(cb);
    //     }
    //     this._ask(msgId);
    //     return this;
    // }

    getStream(cb: TGOnCb<any>, msgId?: string): TGStream<any>
    {
        const stream = this.subscribe();

        (async () =>
        {
            for await (const packet of stream)
            {
                cb(packet);
            }
        })();

        this._ask(msgId);
        return stream;
    }

    receive(value: TGNode|undefined): TGGraphQuery
    {
        this.subscriptions(true).forEach((streamName) =>
        {
            this.publish(streamName, value);
        });
        return this;
    }

    match(node: TGNode|undefined): boolean
    {
        const soul = getNodeSoul(node);

        if (this._listOptions)
        {
            return listFilterMatch(this._listOptions, soul)
        }

        return soul === this.options['#'];
    }

    // on(cb: (data: TGNode|undefined, soul: string) => void): TGGraphQuery
    // {
    //     this._data.on(cb);
    //     return this;
    // }

    off(): TGGraphQuery
    {
        if (isFunction(this._endCurQuery))
        {
            this._endCurQuery();
        }
        this.destroy();

        return this;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _ask(msgId?: string): TGGraphQuery
    {
        if (this._endCurQuery)
        {
            return this;
        }

        const data: TGGet = {
            msgId  : msgId || uuidv4(),
            options: this.options,
            cb     : this._onDirectQueryReply.bind(this)
        };

        this._endCurQuery = this._graph.get(data);
        return this;
    }

    private _onDirectQueryReply(msg: TGMessage): void
    {
        if (!msg.put)
        {
            const soul = msg['#'] || this.options['#'];

            this._updateGraph(
                {
                    [soul]: undefined,
                },
                msg['@'],
            );
        }
    }
}
