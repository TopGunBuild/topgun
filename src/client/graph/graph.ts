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
    TGOnCb,
} from '../../types';
import { TGEvent } from '../control-flow/event';
import { TGGraphConnector } from '../transports/graph-connector';
import { TGGraphNode } from './graph-node';
import {
    diffSets,
    flattenGraphData,
    generateMessageId,
    getPathData,
} from './graph-utils';
import { dataWalking, set } from '../../utils/data-walking';
import { isDefined } from '../../utils/is-defined';

interface TGGraphOptions {
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
        readonly graphData: TGEvent<
            TGGraphData,
            string | undefined,
            string | undefined
        >;
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
    private readonly _nodes: {
        [soul: string]: TGGraphNode;
    };

    /**
     * Constructor
     */
    constructor() 
    {
        this.id = generateMessageId();
        this.receiveGraphData = this.receiveGraphData.bind(this);
        this.__onConnectorStatus = this.__onConnectorStatus.bind(this);
        this.activeConnectors = 0;
        this.events = {
            get: new TGEvent('request soul'),
            graphData: new TGEvent('graph data'),
            off: new TGEvent('off event'),
            put: new TGEvent('put data'),
        };
        this._opt = {};
        this._graph = {};
        this._nodes = {};
        this._connectors = [];
        this._readMiddleware = [];
        this._writeMiddleware = [];
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
     * @param kind Optionaly register write middleware instead of read by passing "write"
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
     * @param kind Optionaly unregister write middleware instead of read by passing "write"
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
     * Read a potentially multi-level deep path from the graph
     *
     * @param path The path to read
     * @param cb The callback to invoke with results
     * @returns a cleanup function to after done with query
     */
    query(path: readonly string[], cb: TGOnCb): () => void 
    {
        let lastSouls = [] as readonly string[];
        let currentValue: TGValue | undefined;

        const updateQuery = () => 
        {
            const { souls, value, complete } = getPathData(path, this._graph);
            const [added, removed] = diffSets(lastSouls, souls);

            if (
                (complete && !isDefined(currentValue)) ||
                (isDefined(value) && value !== currentValue)
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

            lastSouls = souls;
        };

        updateQuery();

        return () => 
        {
            for (const soul of lastSouls) 
            {
                this._unlistenSoul(soul, updateQuery);
            }
        };
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
        data: TGValue,
        cb?: TGMessageCb,
        uuidFn?: (path: readonly string[]) => Promise<string> | string,
        getPub?: string,
        putOpt?: TGOptionsPut,
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
                    'err': err,
                    'ok': false,
                });
            }
            throw err;
        }

        const soul = fullPath.shift() as string;
        const rawData = set(fullPath, data);
        const graphData = dataWalking(rawData, [soul]);

        this.put(graphData, fullPath, cb, undefined, soul, putOpt);
    }

    /**
     * Request node data
     *
     * @param soul identifier of node to request
     * @param cb callback for response messages
     * @param msgId optional unique message identifier
     * @returns a function to cleanup listeners when done
     */
    get(soul: string, cb?: TGMessageCb, msgId?: string): () => void 
    {
        const id = msgId || generateMessageId();

        this.events.get.trigger({
            cb,
            msgId: id,
            soul,
        });

        return () => this.events.off.trigger(id);
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
        data: TGGraphData,
        fullPath: string[],
        cb?: TGMessageCb,
        msgId?: string,
        soul?: string,
        putOpt?: TGOptionsPut,
    ): () => void 
    {
        let diff: TGGraphData | undefined = flattenGraphData(
            addMissingState(data),
        );

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
                msgId: id,
            });

            if (cb) 
            {
                // && this.events.put.listenerCount() === 0
                cb({
                    '#': undefined,
                    '@': msgId,
                    'err': null,
                    'ok': true,
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
     * Synchronously invoke callback function for each connector to this graph
     *
     * @param cb The callback to invoke
     */
    eachConnector(cb: (connector: TGGraphConnector) => void): TGGraph 
    {
        for (const connector of this._connectors) 
        {
            cb(connector);
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

            this._node(soul).receive(
                (this._graph[soul] = mergeNodes(
                    this._graph[soul],
                    diff[soul],
                    this._opt.mutable ? 'mutable' : 'immutable',
                )),
            );
        }

        this.events.graphData.trigger(diff, id, replyToId);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _node(soul: string): TGGraphNode 
    {
        return (this._nodes[soul] =
            this._nodes[soul] ||
            new TGGraphNode(this, soul, this.receiveGraphData));
    }

    private _requestSoul(soul: string, cb: TGNodeListenCb): TGGraph 
    {
        this._node(soul).get(cb);
        return this;
    }

    private _unlistenSoul(soul: string, cb: TGNodeListenCb): TGGraph 
    {
        const node = this._nodes[soul];
        if (!node) 
        {
            return this;
        }
        node.off(cb);
        if (node.listenerCount() <= 0) 
        {
            node.off();
            this._forgetSoul(soul);
        }
        return this;
    }

    private _forgetSoul(soul: string): TGGraph 
    {
        const node = this._nodes[soul];
        if (node) 
        {
            node.off();
            delete this._nodes[soul];
        }
        // delete this._graph[soul];
        return this;
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
