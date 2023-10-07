import { isFunction, isEmptyObject } from '@topgunbuild/typed';
import { TGGet, TGGraphData, TGMessage, TGNode, TGOnCb, TGOptionsGet } from '../../types';
import { TGGraph } from './graph';
import { TGQueryOptions } from '../../storage';
import { listFilterMatch, queryOptionsFromGetOptions } from '../../storage/utils';
import { uuidv4 } from '../../utils/uuidv4';
import { TGExchange } from '../../stream/exchange';
import { TGStream } from '../../stream/stream';

export class TGGraphQuery extends TGExchange
{
    readonly queryString: string;
    readonly options: TGOptionsGet;

    private _endCurQuery?: () => void;
    private readonly _listOptions: TGQueryOptions|null;
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
        this.options             = JSON.parse(queryString);
        this.queryString         = queryString;
        this._graph              = graph;
        this._updateGraph        = updateGraph;
        this._listOptions        = queryOptionsFromGetOptions(this.options)
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    listenerCount(): number
    {
        return this.subscriptions(true).length;
    }

    getStream(cb: TGOnCb<any>, msgId?: string, askOnce?: boolean): TGStream<any>
    {
        const stream = this.subscribe();

        (async () =>
        {
            for await (const packet of stream)
            {
                cb(packet);
            }
        })();

        this.#ask(msgId, askOnce);
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

    match(soul: string): boolean
    {
        if (this._listOptions)
        {
            return listFilterMatch(this._listOptions, soul)
        }

        return soul === this.options['#'];
    }

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

    #onDirectQueryReply(msg: TGMessage): void
    {
        if (isEmptyObject(msg.put))
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
