import { isObject, isString } from 'topgun-typed';
import { AsyncStreamEmitter } from 'topgun-async-stream-emitter';
import { diffCRDT } from '../crdt';
import { TGLink } from './link';
import { TGGraph } from './graph/graph';
import { TGGraphConnector } from './transports/graph-connector';
import { DEFAULT_OPTIONS, TGClientOptions, TGClientPeerOptions } from './client-options';
import { createConnector } from './transports/web-socket-graph-connector';
import { TGSocketClientOptions } from 'topgun-socket/client';
import { TGUserApi } from './user-api';
import { pubFromSoul, unpackGraph } from '../sea';
import { TGIndexedDBConnector } from '../indexeddb/indexeddb-connector';
import { TGSystemEvent, TGNode } from '../types';
import { match } from '../utils/match';
import { wait } from '../utils/wait';
import { assertObject, assertGetPath } from '../utils/assert';

/**
 * Main entry point for TopGun in browser
 */
export class TGClient extends AsyncStreamEmitter<any>
{
    static match = match;

    options: TGClientOptions;
    pub: string|undefined;
    readonly graph: TGGraph;
    protected _user?: TGUserApi;
    readonly WAIT_FOR_USER_PUB: string;

    /**
     * Constructor
     */
    constructor(options?: TGClientOptions)
    {
        super();
        options                = isObject(options) ? options : {};
        this.options           = { ...DEFAULT_OPTIONS, ...options };
        this.graph             = this.options && this.options.graph ? this.options.graph : new TGGraph();
        this.WAIT_FOR_USER_PUB = '__WAIT_FOR_USER_PUB__';

        this.graph.use(diffCRDT);
        this.graph.use(diffCRDT, 'write');

        this.opt(this.options);
        this.registerSeaMiddleware();
        this.user().recoverCredentials();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Get User API
     */
    user(): TGUserApi;
    user(pubOrNode: string|TGNode): TGLink;
    user(pubOrNode?: string|TGNode): TGUserApi|TGLink
    {
        if (pubOrNode)
        {
            if (isObject(pubOrNode) && pubOrNode._ && pubOrNode._['#'])
            {
                this.pub = pubFromSoul(pubOrNode._['#']);
            }
            else if (isString(pubOrNode))
            {
                this.pub = pubOrNode.startsWith('~')
                    ? pubFromSoul(pubOrNode)
                    : pubOrNode;
            }
            else
            {
                throw Error('Argument must be public key or node!');
            }

            return this.get('~' + this.pub);
        }

        return (this._user =
            this._user ||
            new TGUserApi(
                this,
                this.options.sessionStorage,
                this.options.sessionStorageKey,
            ));
    }

    /**
     * Set TopGun configuration options
     */
    opt(options: TGClientOptions): TGClient
    {
        options      = assertObject(options);
        this.options = { ...this.options, ...options };

        if (Array.isArray(options.peers))
        {
            this.handlePeers(options.peers);
        }
        if (options.localStorage)
        {
            this.useConnector(new TGIndexedDBConnector(options.localStorageKey, options));
        }
        if (Array.isArray(options.connectors))
        {
            options.connectors.forEach(connector =>
                this.useConnector(connector),
            );
        }

        return this;
    }

    /**
     * Traverse a location in the graph
     */
    get(soul: string): TGLink
    {
        return new TGLink(this, assertGetPath(soul));
    }

    /**
     * Close all connections
     */
    async disconnect(): Promise<void>
    {
        // Wait for topgun-socket closed all transport gateways
        await wait(5);
        await this.graph.eachConnector(async (connector) =>
        {
            await connector.disconnect();
        });
        this.closeAllListeners();
    }

    /**
     * System events callback
     */
    on(event: TGSystemEvent, cb: (value) => void): void
    {
        (async () =>
        {
            for await (const value of this.listener(event))
            {
                cb(value);
            }
        })();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Register middleware with Security, Encryption, & Authorization - SEA
     */
    private registerSeaMiddleware(): void
    {
        this.graph.use(graph =>
            unpackGraph(
                graph,
                this.graph['_opt'].mutable ? 'mutable' : 'immutable',
            ),
        );
    }

    /**
     * Setup GraphConnector for graph
     */
    private useConnector(connector: TGGraphConnector): void
    {
        connector.sendPutsFromGraph(this.graph);
        connector.sendRequestsFromGraph(this.graph);
        this.graph.connect(connector);
    }

    /**
     * Connect to peers via connector TopGunSocket
     */
    private async handlePeers(peers: TGClientPeerOptions[]): Promise<void>
    {
        peers.forEach((peer: TGClientPeerOptions) =>
        {
            try
            {
                if (isString(peer))
                {
                    const url                            = new URL(peer);
                    const options: TGSocketClientOptions = {
                        hostname: url.hostname,
                        secure  : url.protocol.includes('https'),
                    };

                    if (url.port.length > 0)
                    {
                        options.port = Number(url.port);
                    }

                    this.useConnector(createConnector(options));
                }
                else if (isObject(peer))
                {
                    this.useConnector(createConnector(peer));
                }
            }
            catch (e)
            {
                console.error(e);
            }
        });
    }
}
