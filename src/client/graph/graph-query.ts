import { TGGet, TGGraphData, TGMessage, TGNode, TGNodeListenCb, TGOptionsGet } from '../../types';
import { TGEvent, TGGraph } from '..';
import { getNodeSoul } from '../../utils/node';
import { StorageListOptions } from '../../storage';
import { listFilterMatch, storageListOptionsFromGetOptions } from '../../storage/utils';

export class TGGraphQuery
{
    readonly queryString: string;
    readonly options: TGOptionsGet;

    private _endCurQuery?: () => void;
    private readonly _listOptions: StorageListOptions|null;
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
        queryString: string,
        updateGraph: (data: TGGraphData, replyToId?: string) => void,
    )
    {
        this._onDirectQueryReply = this._onDirectQueryReply.bind(this);
        this.options             = JSON.parse(queryString);
        this.queryString         = queryString;
        this._data               = new TGEvent<TGNode|undefined>(`<GraphNode ${this.queryString}>`);
        this._graph              = graph;
        this._updateGraph        = updateGraph;
        this._listOptions        = storageListOptionsFromGetOptions(this.options)
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    listenerCount(): number
    {
        return this._data.listenerCount();
    }

    get(cb?: TGNodeListenCb): TGGraphQuery
    {
        if (cb)
        {
            this.on(cb);
        }
        this._ask();
        return this;
    }

    receive(data: TGNode|undefined): TGGraphQuery
    {
        this._data.trigger(data, getNodeSoul(data));
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

    on(cb: (data: TGNode|undefined, soul: string) => void): TGGraphQuery
    {
        this._data.on(cb);
        return this;
    }

    off(cb?: (data: TGNode|undefined, soul: string) => void): TGGraphQuery
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

    private _ask(): TGGraphQuery
    {
        if (this._endCurQuery)
        {
            return this;
        }

        const data: TGGet = {
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
