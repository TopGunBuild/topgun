import { addMissingState, mergeNodes } from '../../crdt';
import {
    Get,
    Put,
    GraphData,
    MessageCb,
    Value,
    OptionsPut
} from '../../types';
import { Event } from '../control-flow/event';
import { Middleware, MiddlewareType, NodeListenCb, OnCb } from '../interfaces';
import { GraphConnector } from '../transports/graph-connector';
import { GraphNode } from './graph-node';
import { diffSets, flattenGraphData, generateMessageId, getPathData } from './graph-utils';
import { dataWalking, set } from '../../utils/data-walking';
import { isDefined } from '../../utils/is-defined';

interface GraphOptions
{
    readonly mutable?: boolean;
}

/**
 * High level management of a subset of the graph
 *
 * Provides facilities for querying and writing to graph data from one or more sources
 */
export class Graph
{
    readonly id: string;

    readonly events: {
        readonly graphData: Event<GraphData, string|undefined, string|undefined>
        readonly put: Event<Put>
        readonly get: Event<Get>
        readonly off: Event<string>
    };

    activeConnectors: number;

    private _opt: GraphOptions;
    private readonly _connectors: GraphConnector[];
    private readonly _readMiddleware: Middleware[];
    private readonly _writeMiddleware: Middleware[];
    private readonly _graph: GraphData;
    private readonly _nodes: {
        [soul: string]: GraphNode
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
            get      : new Event('request soul'),
            graphData: new Event('graph data'),
            off      : new Event('off event'),
            put      : new Event('put data')
        };
        this._opt                = {};
        this._graph              = {};
        this._nodes              = {};
        this._connectors         = [];
        this._readMiddleware     = [];
        this._writeMiddleware    = [];
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Configure graph options
     *
     * Currently unused
     *
     * @param options
     */
    opt(options: GraphOptions): Graph
    {
        this._opt = { ...this._opt, ...options };
        return this
    }

    /**
     * Connect to a source/destination for graph data
     *
     * @param connector the source or destination for graph data
     */
    connect(connector: GraphConnector): Graph
    {
        if (this._connectors.indexOf(connector) !== -1)
        {
            return this
        }
        this._connectors.push(connector.connectToGraph(this));

        connector.events.connection.on(this.__onConnectorStatus);
        connector.events.graphData.on(this.receiveGraphData);

        if (connector.isConnected)
        {
            this.activeConnectors++;
        }
        return this
    }

    /**
     * Disconnect from a source/destination for graph data
     *
     * @param connector the source or destination for graph data
     */
    disconnect(connector: GraphConnector): Graph
    {
        const idx = this._connectors.indexOf(connector);
        connector.events.graphData.off(this.receiveGraphData);
        connector.events.connection.off(this.__onConnectorStatus);
        if (idx !== -1)
        {
            this._connectors.splice(idx, 1)
        }
        if (connector.isConnected)
        {
            this.activeConnectors--
        }
        return this
    }

    /**
     * Register graph middleware
     *
     * @param middleware The middleware function to add
     * @param kind Optionaly register write middleware instead of read by passing "write"
     */
    use(
        middleware: Middleware,
        kind = 'read' as MiddlewareType
    ): Graph
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
     * @param kind Optionaly unregister write middleware instead of read by passing "write"
     */
    unuse(
        middleware: Middleware,
        kind = 'read' as MiddlewareType
    ): Graph
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

        return this
    }

    /**
     * Read a potentially multi-level deep path from the graph
     *
     * @param path The path to read
     * @param cb The callback to invoke with results
     * @returns a cleanup function to after done with query
     */
    query(path: readonly string[], cb: OnCb): () => void
    {
        let lastSouls = [] as readonly string[];
        let currentValue: Value|undefined;

        const updateQuery = () =>
        {
            const { souls, value, complete } = getPathData(path, this._graph);
            const [added, removed]           = diffSets(lastSouls, souls);

            if (
                (complete && !isDefined(currentValue)) || (isDefined(value) && value !== currentValue)
            )
            {
                currentValue = value;
                cb(value, path[path.length - 1]);
            }

            for (const soul of added)
            {
                this._requestSoul(soul, updateQuery);
            }

            for (const soul of removed)
            {
                this._unlistenSoul(soul, updateQuery);
            }

            lastSouls = souls
        };

        updateQuery();

        return () =>
        {
            for (const soul of lastSouls)
            {
                this._unlistenSoul(soul, updateQuery)
            }
        }
    }

    /**
     * Write graph data to a potentially multi-level deep path in the graph
     *
     * @param fullPath The path to read
     * @param data The value to write
     * @param cb Callback function to be invoked for write acks
     * @param uuidFn
     * @param getPub
     * @param putOpt
     * @returns a promise
     */
    async putPath(
        fullPath: string[],
        data: Value,
        cb?: MessageCb,
        uuidFn?: (path: readonly string[]) => Promise<string>|string,
        getPub?: string,
        putOpt?: OptionsPut
    ): Promise<void>
    {
        if (!fullPath.length)
        {
            const err = new Error('No path specified');
            if (cb)
            {
                cb({
                    '#': undefined,
                    '@': '',
                    err: err,
                    ok : false
                });
            }
            throw err;
        }

        const soul      = fullPath.shift();
        const rawData   = set(fullPath, data);
        const graphData = dataWalking(rawData, [soul]);

        this.put(graphData, fullPath, cb, null, soul, putOpt);
    }

    /**
     * Request node data
     *
     * @param soul identifier of node to request
     * @param cb callback for response messages
     * @param msgId optional unique message identifier
     * @returns a function to cleanup listeners when done
     */
    get(soul: string, cb?: MessageCb, msgId?: string): () => void
    {
        const id = msgId || generateMessageId();

        this.events.get.trigger({
            cb,
            msgId: id,
            soul
        });

        return () => this.events.off.trigger(id)
    }

    /**
     * Write node data
     *
     * @param data one or more nodes keyed by soul
     * @param fullPath The path to read
     * @param cb optional callback for response messages
     * @param msgId optional unique message identifier
     * @param soul string
     * @param putOpt put options
     * @returns a function to clean up listeners when done
     */
    put(
        data: GraphData,
        fullPath: string[],
        cb?: MessageCb,
        msgId?: string,
        soul?: string,
        putOpt?: OptionsPut
    ): () => void
    {
        let diff: GraphData|undefined = flattenGraphData(addMissingState(data));

        const id = msgId || generateMessageId();
        (async () =>
        {
            for (const fn of this._writeMiddleware)
            {
                if (!diff)
                {
                    return;
                }
                diff = await fn(diff, this._graph, putOpt, fullPath);
            }

            if (!diff)
            {
                return;
            }

            await this.receiveGraphData(diff);

            this.events.put.trigger({
                cb,
                graph: diff,
                msgId: id
            });

            if (cb) // && this.events.put.listenerCount() === 0
            {
                cb({
                    '#': undefined,
                    '@': msgId,
                    err: null,
                    ok : true
                });
            }
        })();

        return () => this.events.off.trigger(id)
    }

    connectorCount(): number
    {
        return this._connectors.length;
    }

    /**
     * Synchronously invoke callback function for each connector to this graph
     *
     * @param cb The callback to invoke
     */
    eachConnector(cb: (connector: GraphConnector) => void): Graph
    {
        for (const connector of this._connectors)
        {
            cb(connector)
        }

        return this
    }

    /**
     * Update graph data in this chain from some local or external source
     */
    async receiveGraphData(
        data?: GraphData,
        id?: string,
        replyToId?: string
    ): Promise<void>
    {
        let diff = data;

        for (const fn of this._readMiddleware)
        {
            if (!diff)
            {
                return;
            }
            diff = await fn(diff, this._graph)
        }

        if (!diff)
        {
            return;
        }

        for (const soul in diff)
        {
            if (!soul)
            {
                continue
            }

            this._node(soul).receive(
                (this._graph[soul] = mergeNodes(
                    this._graph[soul],
                    diff[soul],
                    this._opt.mutable ? 'mutable' : 'immutable'
                ))
            )
        }

        this.events.graphData.trigger(diff, id, replyToId);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _node(soul: string): GraphNode
    {
        return (this._nodes[soul] = this._nodes[soul] || new GraphNode(this, soul, this.receiveGraphData));
    }

    private _requestSoul(soul: string, cb: NodeListenCb): Graph
    {
        this._node(soul).get(cb);
        return this
    }

    private _unlistenSoul(soul: string, cb: NodeListenCb): Graph
    {
        const node = this._nodes[soul];
        if (!node)
        {
            return this
        }
        node.off(cb);
        if (node.listenerCount() <= 0)
        {
            node.off();
            this._forgetSoul(soul);
        }
        return this
    }

    private _forgetSoul(soul: string): Graph
    {
        const node = this._nodes[soul];
        if (node)
        {
            node.off();
            delete this._nodes[soul]
        }
        // delete this._graph[soul];
        return this
    }

    private __onConnectorStatus(connected?: boolean): void
    {
        if (connected)
        {
            this.activeConnectors++
        }
        else
        {
            this.activeConnectors--
        }
    }
}
