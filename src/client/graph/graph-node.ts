import { TGGet, TGGraphData, TGMessage, TGNode, TGNodeListenCb, TGOptionsGet } from '../../types';
import { TGEvent } from '../control-flow/event';
import { TGGraph } from './graph';

/**
 * Query state around a single node in the graph
 */
export class TGGraphNode
{
    readonly soul: string;
    readonly optionsGet: TGOptionsGet;

    private _endCurQuery?: () => void;
    private readonly _data: TGEvent<TGNode|undefined>;
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
        optionsGet: TGOptionsGet,
        updateGraph: (data: TGGraphData, replyToId?: string) => void,
    )
    {
        this._onDirectQueryReply = this._onDirectQueryReply.bind(this);
        this.soul                = optionsGet['#'];
        this.optionsGet          = optionsGet;
        this._graph              = graph;
        this._updateGraph        = updateGraph;
        this._data               = new TGEvent<TGNode|undefined>(`<GraphNode ${soul}>`);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    listenerCount(): number
    {
        return this._data.listenerCount();
    }

    get(cb?: TGNodeListenCb): TGGraphNode
    {
        if (cb)
        {
            this.on(cb);
        }
        this._ask();
        return this;
    }

    receive(data: TGNode|undefined): TGGraphNode
    {
        this._data.trigger(data, this.soul);
        return this;
    }

    on(cb: (data: TGNode|undefined, soul: string) => void): TGGraphNode
    {
        this._data.on(cb);
        return this;
    }

    off(cb?: (data: TGNode|undefined, soul: string) => void): TGGraphNode
    {
        if (cb)
        {
            this._data.off(cb);
        }
        else
        {
            this._data.reset();
        }

        if (this._endCurQuery && !this._data.listenerCount())
        {
            this._endCurQuery();
            this._endCurQuery = undefined;
        }

        return this;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _ask(): TGGraphNode
    {
        if (this._endCurQuery)
        {
            return this;
        }

        const data: TGGet = {
            soul: this.soul,
            cb  : this._onDirectQueryReply.bind(this)
        };

        if (opts)
        {
            data.opts = opts;
        }

        this._endCurQuery = this._graph.get(data);
        return this;
    }

    private _onDirectQueryReply(msg: TGMessage): void
    {
        if (!msg.put)
        {
            this._updateGraph(
                {
                    [this.soul]: undefined,
                },
                msg['@'],
            );
        }
    }
}
