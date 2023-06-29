import { isUndefined, isDefined } from 'topgun-typed';
import { addMissingState, mergeNodes } from '../../crdt';
import {
    TGGet,
    TGPut,
    TGGraphData,
    TGMessageCb,
    TGValue,
    TGOptionsPut,
    TGMiddleware,
    TGMiddlewareType,
    TGNodeListenCb,
    TGOnCb, TGOptionsGet,
} from '../../types';
import { TGEvent } from '../control-flow/event';
import { TGGraphConnector } from '../transports/graph-connector';
import {
    diffSets,
    generateMessageId, getNodesFromGraph,
    getPathData,
    flattenGraphData
} from './graph-utils';
import { getNodeSoul } from '../../utils/node';
import { TGGraphQuery } from './graph-query';
import { stringifyOptionsGet } from '../../utils/stringify-options-get';

interface TGGraphOptions
{
    readonly mutable?: boolean;
}

/**
 * High level management of a subset of the graph
 *
 * Provides facilities for querying and writing to graph data from one or more sources
 */
export class TGGraph
{
    readonly id: string;

    readonly events: {
        readonly graphData: TGEvent<TGGraphData,
        string|undefined,
        string|undefined>;
        readonly put: TGEvent<TGPut>;
        readonly get: TGEvent<TGGet>;
        readonly off: TGEvent<string>;
    };

    activeConnectors: number;

    private _opt: TGGraphOptions;
    private readonly _connectors: TGGraphConnector[];
    private readonly _readMiddleware: TGMiddleware[];
    private readonly _writeMiddleware: TGMiddleware[];
    private readonly _graph: TGGraphData;
    private readonly _queries: {
        [queryString: string]: TGGraphQuery;
    };

    /**
     * Constructor
     */
    constructor()
    {
        this.id                  = generateMessageId();
        this.receiveGraphData    = this.receiveGraphData.bind(this);
        this.__onConnectorStatus = this.__onConnectorStatus.bind(this);
        this.activeConnectors    = 0;
        this.events              = {
            get      : new TGEvent('request soul'),
            graphData: new TGEvent('graph data'),
            off      : new TGEvent('off event'),
            put      : new TGEvent('put data'),
        };
        this._opt                = {};
        this._graph              = {};
        this._queries            = {};
        this._connectors         = [];
        this._readMiddleware     = [];
        this._writeMiddleware    = [];
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Configure graph options
     */
    opt(options: TGGraphOptions): TGGraph
    {
        this._opt = { ...this._opt, ...options };
        return this;
    }

    /**
     * Connect to a source/destination for graph data
     *
     * @param connector the source or destination for graph data
     */
    connect(connector: TGGraphConnector): TGGraph
    {
        if (this._connectors.indexOf(connector) !== -1)
        {
            return this;
        }
        this._connectors.push(connector.connectToGraph(this));

        connector.events.connection.on(this.__onConnectorStatus);
        connector.events.graphData.on(this.receiveGraphData);

        if (connector.isConnected)
        {
            this.activeConnectors++;
        }
        return this;
    }

    /**
     * Disconnect from a source/destination for graph data
     *
     * @param connector the source or destination for graph data
     */
    disconnect(connector: TGGraphConnector): TGGraph
    {
        const idx = this._connectors.indexOf(connector);
        connector.events.graphData.off(this.receiveGraphData);
        connector.events.connection.off(this.__onConnectorStatus);
        if (idx !== -1)
        {
            this._connectors.splice(idx, 1);
        }
        if (connector.isConnected)
        {
            this.activeConnectors--;
        }
        return this;
    }

    /**
     * Register graph middleware
     *
     * @param middleware The middleware function to add
     * @param kind optionally register write middleware instead of read by passing "write"
     */
    use(middleware: TGMiddleware, kind = 'read' as TGMiddlewareType): TGGraph
    {
        if (kind === 'read')
        {
            this._readMiddleware.push(middleware);
        }
        else if (kind === 'write')
        {
            this._writeMiddleware.push(middleware);
        }
        return this;
    }

    /**
     * Unregister graph middleware
     *
     * @param middleware The middleware function to remove
     * @param kind optionally unregister write middleware instead of read by passing "write"
     */
    unuse(
        middleware: TGMiddleware,
        kind = 'read' as TGMiddlewareType,
    ): TGGraph
    {
        if (kind === 'read')
        {
            const idx = this._readMiddleware.indexOf(middleware);
            if (idx !== -1)
            {
                this._readMiddleware.splice(idx, 1);
            }
        }
        else if (kind === 'write')
        {
            const idx = this._writeMiddleware.indexOf(middleware);
            if (idx !== -1)
            {
                this._writeMiddleware.splice(idx, 1);
            }
        }

        return this;
    }

    /**
     * Read a matching nodes from the graph
     */
    queryMany(opts: TGOptionsGet, cb: TGOnCb, msgId?: string): () => void
    {
        getNodesFromGraph(opts, this._graph).forEach((node) =>
        {
            cb(node, getNodeSoul(node));
        });

        const queryString = stringifyOptionsGet(opts);

        this._listen(queryString, cb, msgId);

        return () =>
        {
            this._unlisten(queryString, cb);
        };
    }

    /**
     * Read a potentially multi-level deep path from the graph
     */
    query(path: string[], cb: TGOnCb): () => void
    {
        let lastSouls = [] as string[];
        let currentValue: TGValue|undefined;

        const updateQuery = () =>
        {
            const { souls, value, complete } = getPathData(path, this._graph);
            const [added, removed]           = diffSets(lastSouls, souls);

            if (
                (complete && isUndefined(currentValue)) ||
                (isDefined(value) && value !== currentValue)
            )
            {
                currentValue = value;
                cb(value, path[path.length - 1]);
            }

            for (const soul of added)
            {
                this._listen(this._queryStringBySoul(soul), updateQuery);
            }

            for (const soul of removed)
            {
                this._unlisten(this._queryStringBySoul(soul), updateQuery);
            }

            lastSouls = souls;
        };

        updateQuery();

        return () =>
        {
            for (const soul of lastSouls)
            {
                this._unlisten(this._queryStringBySoul(soul), updateQuery);
            }
        };
    }

    /**
     * Request node data
     */
    get(data: TGGet): () => void
    {
        const msgId = data.msgId || generateMessageId();

        this.events.get.trigger({ ...data, msgId });

        return () => this.events.off.trigger(msgId);
    }

    /**
     * Write graph data to a potentially multi-level deep path in the graph
     *
     * @param fullPath The path to read
     * @param data The value to write
     * @param cb Callback function to be invoked for write acks
     * @param putOpt
     */
    putPath(
        fullPath: string[],
        data: TGValue,
        cb?: TGMessageCb,
        putOpt?: TGOptionsPut,
    ): void
    {
        if (!fullPath.length)
        {
            const err = new Error('No path specified');
            if (cb)
            {
                cb({
                    '#'  : undefined,
                    '@'  : '',
                    'err': err,
                    'ok' : false,
                });
            }
            throw err;
        }

        const { graphData, soul } = flattenGraphData(data, fullPath);

        this.put(graphData, cb, soul, putOpt);
    }

    /**
     * Write node data
     *
     * @param data one or more nodes keyed by soul
     * @param cb optional callback for response messages
     * @param msgId optional unique message identifier
     * @param soul string
     * @param putOpt put options
     * @returns a function to clean up listeners when done
     */
    put(
        data: TGGraphData,
        cb?: TGMessageCb,
        soul?: string,
        putOpt?: TGOptionsPut,
        msgId?: string
    ): () => void
    {
        let diff: TGGraphData = addMissingState(data);

        const id = msgId || generateMessageId();
        (async () =>
        {
            for (const fn of this._writeMiddleware)
            {
                if (!diff)
                {
                    return;
                }
                diff = await fn(diff, this._graph, putOpt);
            }

            if (!diff)
            {
                return;
            }

            await this.receiveGraphData(diff);

            this.events.put.trigger({
                cb,
                graph: diff,
                msgId: id,
            });

            if (cb)
            {
                cb({
                    '#'  : soul,
                    '@'  : msgId,
                    'err': null,
                    'ok' : true,
                });
            }
        })();

        return () => this.events.off.trigger(id);
    }

    connectorCount(): number
    {
        return this._connectors.length;
    }

    /**
     * Invoke callback function for each connector to this graph
     */
    async eachConnector(cb: (connector: TGGraphConnector) => void): Promise<TGGraph>
    {
        for (let index = 0; index < this._connectors.length; index++)
        {
            await cb(this._connectors[index]);
        }

        return this;
    }

    /**
     * Update graph data in this chain from some local or external source
     */
    async receiveGraphData(
        data?: TGGraphData,
        id?: string,
        replyToId?: string,
    ): Promise<void>
    {
        let diff = data;

        for (const fn of this._readMiddleware)
        {
            if (!diff)
            {
                return;
            }
            diff = await fn(diff, this._graph);
        }

        if (!diff)
        {
            return;
        }

        for (const soul in diff)
        {
            if (!soul)
            {
                continue;
            }

            const node = this._graph[soul] = mergeNodes(
                this._graph[soul],
                diff[soul],
                this._opt.mutable ? 'mutable' : 'immutable',
            );

            this._eachQuery((query) =>
            {
                if (query.match(node))
                {
                    query.receive(node);
                }
            });
        }

        this.events.graphData.trigger(diff, id, replyToId);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _query(queryString: string): TGGraphQuery
    {
        return (this._queries[queryString] =
            this._queries[queryString] ||
            new TGGraphQuery(this, queryString, this.receiveGraphData));
    }

    private _listen(queryString: string, cb: TGNodeListenCb, msgId?: string): TGGraph
    {
        this._query(queryString).get(cb, msgId);
        return this;
    }

    private _unlisten(queryString: string, cb: TGNodeListenCb): TGGraph
    {
        const node = this._queries[queryString];
        if (!node)
        {
            return this;
        }
        node.off(cb);
        if (node.listenerCount() <= 0)
        {
            node.off();
            this._forgetQuery(queryString);
        }
        return this;
    }

    private _forgetQuery(queryString: string): TGGraph
    {
        const query = this._queries[queryString];
        if (query)
        {
            query.off();
            delete this._queries[queryString];
        }
        // delete this._graph[soul];
        return this;
    }

    private async _eachQuery(cb: (query: TGGraphQuery) => void): Promise<TGGraph>
    {
        for (const queryString in this._queries)
        {
            await cb(this._queries[queryString]);
        }

        return this;
    }

    private _queryStringBySoul(soul: string): string
    {
        return stringifyOptionsGet({ ['#']: soul });
    }

    private __onConnectorStatus(connected?: boolean): void
    {
        if (connected)
        {
            this.activeConnectors++;
        }
        else
        {
            this.activeConnectors--;
        }
    }
}
