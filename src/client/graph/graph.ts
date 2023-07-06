import { isUndefined, isDefined } from 'topgun-typed';
import { AsyncStreamEmitter } from 'topgun-async-stream-emitter';
import { addMissingState, mergeNodes } from '../../crdt';
import {
    TGGet,
    TGGraphData,
    TGMessageCb,
    TGValue,
    TGOptionsPut,
    TGMiddleware,
    TGMiddlewareType,
    TGOnCb, TGOptionsGet,
} from '../../types';
import { TGGraphConnector } from '../transports/graph-connector';
import {
    diffSets,
    getNodesFromGraph,
    getPathData,
    flattenGraphData
} from './graph-utils';
import { getNodeSoul } from '../../utils/node';
import { TGGraphQuery } from './graph-query';
import { stringifyOptionsGet } from '../../utils/stringify-options-get';
import { uuidv4 } from '../../utils/uuidv4';
import { TGStream } from '../../stream/stream';

interface TGGraphOptions
{
    readonly mutable?: boolean;
}

/**
 * High level management of a subset of the graph
 *
 * Provides facilities for querying and writing to graph data from one or more sources
 */
export class TGGraph extends AsyncStreamEmitter<any>
{
    readonly id: string;

    activeConnectors: number;
    readonly connectors: TGGraphConnector[];

    private _opt: TGGraphOptions;
    private readonly _readMiddleware: TGMiddleware[];
    private readonly _writeMiddleware: TGMiddleware[];
    private readonly _graph: TGGraphData;
    private readonly _queries: {
        [queryString: string]: TGGraphQuery;
    };
    private readonly rootEventEmitter: AsyncStreamEmitter<any>;

    /**
     * Constructor
     */
    constructor(
        rootEventEmitter: AsyncStreamEmitter<any>
    )
    {
        super();
        this.id                  = uuidv4();
        this.receiveGraphData    = this.receiveGraphData.bind(this);
        this.__onConnectorStatus = this.__onConnectorStatus.bind(this);
        this.activeConnectors    = 0;
        this._opt                = {};
        this._graph              = {};
        this._queries            = {};
        this.connectors          = [];
        this._readMiddleware     = [];
        this._writeMiddleware    = [];
        this.rootEventEmitter    = rootEventEmitter;
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
        if (this.connectors.indexOf(connector) !== -1)
        {
            return this;
        }
        this.connectors.push(connector.connectToGraph(this));

        (async () =>
        {
            for await (const value of connector.listener('connect'))
            {
                this.__onConnectorStatus(true);
                this.rootEventEmitter.emit('connectorConnected', connector);
            }
        })();

        (async () =>
        {
            for await (const value of connector.listener('disconnect'))
            {
                this.__onConnectorStatus(false);
                this.rootEventEmitter.emit('connectorDisconnected', connector);
            }
        })();

        (async () =>
        {
            for await (const { data, id, replyToId } of connector.listener('graphData'))
            {
                this.receiveGraphData(data, id, replyToId);
            }
        })();

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
        const idx = this.connectors.indexOf(connector);
        connector.closeAllListeners();
        if (idx !== -1)
        {
            this.connectors.splice(idx, 1);
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
    queryMany<T extends TGValue>(opts: TGOptionsGet, cb: TGOnCb<T>, msgId: string): () => void
    {
        getNodesFromGraph(opts, this._graph).forEach((node) =>
        {
            cb(node as T, getNodeSoul(node));
        });

        const queryString = stringifyOptionsGet(opts);
        const stream      = this._listen(queryString, cb, msgId);

        return () =>
        {
            this._unlisten(queryString, stream);
        };
    }

    /**
     * Read a potentially multi-level deep path from the graph
     */
    query<T extends TGValue>(path: string[], cb: TGOnCb<T>, msgId: string): () => void
    {
        let lastSouls   = [] as string[];
        let currentValue: TGValue|undefined;
        const streamMap = new Map<string, TGStream<any>>();

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
                cb(value as T, path[path.length - 1]);
            }

            for (const soul of added)
            {
                const stream = this._listen(this._queryStringBySoul(soul), updateQuery, msgId);
                streamMap.set(soul, stream);
            }

            for (const soul of removed)
            {
                this._unlisten(this._queryStringBySoul(soul), streamMap.get(soul));
            }

            lastSouls = souls;
        };

        updateQuery();

        return () =>
        {
            for (const soul of lastSouls)
            {
                this._unlisten(this._queryStringBySoul(soul), streamMap.get(soul));
            }
        };
    }

    /**
     * Request node data
     */
    get(data: TGGet): () => void
    {
        const msgId = data.msgId || uuidv4();

        this.emit('get', { ...data, msgId });

        return () => this.emit('off', msgId);
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

        const id = msgId || uuidv4();
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

            this.emit('put', {
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

        return () => this.emit('off', id);
    }

    /**
     * Invoke callback function for each connector to this graph
     */
    async eachConnector(cb: (connector: TGGraphConnector) => void): Promise<TGGraph>
    {
        for (let index = 0; index < this.connectors.length; index++)
        {
            await cb(this.connectors[index]);
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

        this.emit('graphData', { diff, id, replyToId });
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

    private _listen<T extends TGValue>(queryString: string, cb: TGOnCb<T>, msgId?: string): TGStream<any>
    {
        return this._query(queryString).getStream(cb, msgId);
    }

    private _unlisten(queryString: string, stream: TGStream<any>): TGGraph
    {
        if (stream instanceof TGStream)
        {
            stream.destroy();
        }
        const query = this._queries[queryString];
        if (query instanceof TGGraphQuery && query.listenerCount() <= 0)
        {
            query.off();
            delete this._queries[queryString];
        }
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
