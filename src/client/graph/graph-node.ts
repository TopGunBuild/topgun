import { TGGraphData, TGMessage, TGNode, TGNodeListenCb } from '../../types';
import { TGEvent } from '../control-flow/event';
import { TGGraph } from './graph';

/**
 * Query state around a single node in the graph
 */
export class TGGraphNode
{
    public readonly soul: string;

    private _endCurQuery?: () => void;
    private readonly _data: TGEvent<TGNode|undefined>;
    private readonly _graph: TGGraph;
    private readonly _updateGraph: (
        data: TGGraphData,
        replyToId?: string
    ) => void;

    constructor(
        graph: TGGraph,
        soul: string,
        updateGraph: (data: TGGraphData, replyToId?: string) => void
    )
    {
        this._onDirectQueryReply = this._onDirectQueryReply.bind(this);
        this._data               = new TGEvent<TGNode|undefined>(`<GraphNode ${soul}>`);
        this._graph              = graph;
        this._updateGraph        = updateGraph;
        this.soul                = soul;
    }

    public listenerCount(): number
    {
        return this._data.listenerCount()
    }

    public get(cb?: TGNodeListenCb): TGGraphNode
    {
        if (cb)
        {
            this.on(cb)
        }
        this._ask();
        return this
    }

    public receive(data: TGNode|undefined): TGGraphNode
    {
        this._data.trigger(data, this.soul);
        return this
    }

    public on(
        cb: (data: TGNode|undefined, soul: string) => void
    ): TGGraphNode
    {
        this._data.on(cb);
        return this
    }

    public off(
        cb?: (data: TGNode|undefined, soul: string) => void
    ): TGGraphNode
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

    private _ask(): TGGraphNode
    {
        if (this._endCurQuery)
        {
            return this
        }

        this._endCurQuery = this._graph.get(this.soul, this._onDirectQueryReply);
        return this;
    }

    private _onDirectQueryReply(msg: TGMessage): void
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
