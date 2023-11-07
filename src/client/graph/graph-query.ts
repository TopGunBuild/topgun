import { isFunction, isEmptyObject, isDefined } from '@topgunbuild/typed';
import { TGGet, TGGraphData, TGMessage, TGNode, TGOnCb, TGOptionsGet, TGValue } from '../../types';
import { TGGraph } from './graph';
import { filterMatch } from '../../storage/utils';
import { uuidv4 } from '../../utils/uuidv4';
import { TGExchange } from '../../stream/exchange';
import { TGStream } from '../../stream/stream';

export class TGGraphQuery extends TGExchange
{
    readonly queryString: string;
    readonly options: TGOptionsGet;

    private _endCurQuery?: () => void;
    protected readonly _isCollectionQuery: boolean;
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
        this.options            = JSON.parse(queryString);
        this.queryString        = queryString;
        this._graph             = graph;
        this._updateGraph       = updateGraph;
        this._isCollectionQuery = isDefined(this.options['%']);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Number of subscribers for this query
     */
    listenerCount(): number
    {
        return this.subscriptions(true).length;
    }

    /**
     * Create a data stream for this query
     */
    getStream(cb: TGOnCb<any>, msgId?: string, askOnce?: boolean): TGStream<any>
    {
        const stream = this.subscribe<{value: TGValue, key: string}>();

        (async () =>
        {
            for await (const { value, key } of stream)
            {
                cb(value, key);
            }
        })();

        this.#ask(msgId, askOnce);
        return stream;
    }

    /**
     * Publish data to all subscriptions
     */
    receive(value: TGNode|undefined, soul: string): TGGraphQuery
    {
        this.subscriptions(true).forEach((streamName) =>
        {
            this.publish(streamName, { value, key: soul });
        });
        return this;
    }

    /**
     * Yes, if the request is subscribed to this soul
     */
    match(soul: string): boolean
    {
        return filterMatch(soul, this.options);
    }

    /**
     * Destroy request
     */
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

    /**
     * Request data from peers
     */
    #ask(msgId?: string, once?: boolean): TGGraphQuery
    {
        if (this._endCurQuery)
        {
            return this;
        }

        const data: TGGet = {
            once,
            msgId  : msgId || uuidv4(),
            options: this.options,
            cb     : (msg: TGMessage) => this.#onDirectQueryReply(msg)
        };

        this._endCurQuery = this._graph.get(data);
        return this;
    }

    /**
     * Processing a direct peer response
     */
    #onDirectQueryReply(msg: TGMessage): void
    {
        // Return an empty response when requesting a node or property
        if (isEmptyObject(msg.put) && !this._isCollectionQuery)
        {
            const soul = this.options['#'];

            this._updateGraph(
                {
                    [soul]: undefined,
                },
                msg['@'],
            );
        }
    }
}
