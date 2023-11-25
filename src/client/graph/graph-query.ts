import { isFunction, isEmptyObject, isDefined, isNull } from '@topgunbuild/typed';
import { TGGet, TGGraphData, TGMessage, TGNode, TGOnCb, TGOptionsGet, TGRefNode, TGValue } from '../../types';
import { TGGraph } from './graph';
import { filterMatch } from '../../storage/utils';
import { uuidv4 } from '../../utils/uuidv4';
import { TGExchange } from '../../stream/exchange';
import { TGStream } from '../../stream/stream';
import { getNodeSoul } from '../../utils';

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

    readonly targetNodesMap: Map<string, string>;
    readonly referenceNodesMap: Map<string, string>;
    readonly streamMap: Map<string, TGStream<any>>;

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
        this.targetNodesMap     = new Map<string, string>();
        this.referenceNodesMap  = new Map<string, string>();
        this.streamMap          = new Map<string, TGStream<any>>();
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
    getStream(cb?: TGOnCb<any>, msgId?: string, askOnce?: boolean): TGStream<any>
    {
        const stream = this.subscribe<{value: TGValue, key: string}>();

        if (isFunction(cb))
        {
            (async () =>
            {
                for await (const { value, key } of stream)
                {
                    cb(value, key);
                }
            })();
        }

        this.#ask(msgId, askOnce);
        return stream;
    }

    /**
     * Receive reference node
     */
    setRef(node: TGRefNode): void
    {
        const soul = getNodeSoul(node);

        if (!this.targetNodesMap.has(node['#']))
        {
            this.targetNodesMap.set(node['#'], soul);
            this.referenceNodesMap.set(soul, node['#']);
        }
    }

    /**
     * Receive reference target node
     */
    receiveTarget(node: TGNode|undefined, soul: string): void
    {
        this.#publishNode(node, soul);
    }

    /**
     * Receive data from some local or external source
     */
    receive(node: TGNode|undefined, soul: string): void
    {
        if (isNull(node) && this.referenceNodesMap.has(soul))
        {
            this.targetNodesMap.delete(this.referenceNodesMap.get(soul));
            this.referenceNodesMap.delete(soul);
        }

        this.#publishNode(node, soul);
    }

    /**
     * If there is a reference target node
     */
    isTarget(soul: string): boolean
    {
        return this.targetNodesMap.has(soul);
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
        this.referenceNodesMap.clear();
        this.targetNodesMap.clear();

        for (const soul of this.streamMap.keys())
        {
            this._graph.unlisten(this._graph.queryStringForSoul(soul), this.streamMap.get(soul));
        }
        this.streamMap.clear();

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
     * Publish node to all subscriptions
     */
    #publishNode(value: TGNode|undefined, soul: string): void
    {
        // if (soul.includes('/node/') && this.queryString.includes('favorite'))
        // {
        //     console.trace('publishNode-', { soul, value });
        // }
        this.subscriptions(true).forEach((streamName) =>
        {
            this.publish(streamName, { value, key: soul });
        });
    }

    /**
     * Processing a direct peer response
     */
    #onDirectQueryReply(msg: TGMessage): void
    {
        // Return an empty response when requesting a node or property
        // if (isEmptyObject(msg.put) && !this._isCollectionQuery)
        // {
        //     const soul = this.options['#'];
        //
        //     this._updateGraph(
        //         {
        //             [soul]: undefined,
        //         },
        //         msg['@'],
        //     );
        // }
    }
}
