import { GraphData, Message, Node, NodeListenCb } from '../../types';
import { Event } from '../control-flow/event';
import { Graph } from './graph';

/**
 * Query state around a single node in the graph
 */
export class GraphNode
{
    public readonly soul: string;

    private _endCurQuery?: () => void;
    private readonly _data: Event<Node|undefined>;
    private readonly _graph: Graph;
    private readonly _updateGraph: (
        data: GraphData,
        replyToId?: string
    ) => void;

    constructor(
        graph: Graph,
        soul: string,
        updateGraph: (data: GraphData, replyToId?: string) => void
    )
    {
        this._onDirectQueryReply = this._onDirectQueryReply.bind(this);
        this._data               = new Event<Node|undefined>(`<GraphNode ${soul}>`);
        this._graph              = graph;
        this._updateGraph        = updateGraph;
        this.soul                = soul;
    }

    public listenerCount(): number
    {
        return this._data.listenerCount()
    }

    public get(cb?: NodeListenCb): GraphNode
    {
        if (cb)
        {
            this.on(cb)
        }
        this._ask();
        return this
    }

    public receive(data: Node|undefined): GraphNode
    {
        this._data.trigger(data, this.soul);
        return this
    }

    public on(
        cb: (data: Node|undefined, soul: string) => void
    ): GraphNode
    {
        this._data.on(cb);
        return this
    }

    public off(
        cb?: (data: Node|undefined, soul: string) => void
    ): GraphNode
    {
        if (cb)
        {
            this._data.off(cb)
        }
        else
        {
            this._data.reset()
        }

        if (this._endCurQuery && !this._data.listenerCount())
        {
            this._endCurQuery();
            this._endCurQuery = undefined
        }

        return this
    }

    private _ask(): GraphNode
    {
        if (this._endCurQuery)
        {
            return this
        }

        this._endCurQuery = this._graph.get(this.soul, this._onDirectQueryReply);
        return this;
    }

    private _onDirectQueryReply(msg: Message): void
    {
        if (!msg.put)
        {
            this._updateGraph(
                {
                    [this.soul]: undefined
                },
                msg['@']
            )
        }
    }
}
