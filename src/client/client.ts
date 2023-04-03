import { diffCRDT } from '../crdt'
import { Link } from './link'
import { Graph } from './graph/graph'
import { GraphConnector } from './transports/graph-connector';
import { ClientOptions } from './client-options';
import { createConnector } from '../socket-connector';
import { ClientOptions as SocketClientOptions } from 'topgun-socket';
import { UserApi } from './user-api';
import { pubFromSoul, unpackGraph } from '../sea';
import { polyfillGlobalThis } from '../utils/global-this';
import { isObject } from '../utils/is-object';
import { IndexedDbConnector } from '../indexeddb/indexeddb-connector';
import { localStorageAdapter } from '../utils/local-storage';
import { OnCb, Node, UserReference } from '../types';
import { Event } from './control-flow/event';
import { isString } from '../utils/is-string';
import { LexLink } from './lex-link';
import { match } from '../utils/match';

polyfillGlobalThis(); // Make "globalThis" available

const DEFAULT_OPTIONS: Required<ClientOptions> = {
    peers            : [],
    graph            : new Graph(),
    connectors       : [],
    persistStorage   : false,
    storageKey       : 'top-gun-nodes',
    persistSession   : true,
    sessionStorage   : localStorageAdapter,
    sessionStorageKey: 'top-gun-session',
    passwordMinLength: 8,
};

/**
 * Main entry point for TopGun
 *
 * Usage:
 *
 *   const topGun = new TopGun.Client({ peers: ["https://top-gun.io/topgun"]})
 *   topGun.get("topgun/things/f8c3de3d-1fea-4d7c-a8b0-29f63c4c3454").on(thing => console.log(thing))
 */
export class Client
{
    static match = match;

    options: ClientOptions;
    pub: string|undefined;
    readonly graph: Graph;
    protected readonly _authEvent: Event<UserReference>;
    protected _user?: UserApi;

    /**
     * Constructor
     */
    constructor(options?: ClientOptions)
    {
        options         = isObject(options) ? options : {};
        this.options    = { ...DEFAULT_OPTIONS, ...options };
        this._authEvent = new Event<UserReference>('auth data');

        if (this.options && this.options.graph)
        {
            this.graph = this.options.graph;
        }
        else
        {
            this.graph = new Graph();
            this.graph.use(diffCRDT);
            this.graph.use(diffCRDT, 'write');
        }

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
    user(): UserApi
    user(pubOrNode: string|Node): Link
    user(pubOrNode?: string|Node): UserApi|Link
    {
        if (pubOrNode)
        {
            if (isObject(pubOrNode) && pubOrNode._ && pubOrNode._['#'])
            {
                this.pub = pubFromSoul(pubOrNode._['#']);
            }
            else if (isString(pubOrNode))
            {
                this.pub = pubOrNode.startsWith('~') ? pubFromSoul(pubOrNode) : pubOrNode;
            }

            return this.get('~' + this.pub);
        }

        return (
            this._user = this._user || new UserApi(
                this,
                this.options.persistSession,
                this.options.sessionStorage,
                this.options.sessionStorageKey,
                this._authEvent
            )
        );
    }

    /**
     * Set TopGun configuration options
     */
    opt(options: ClientOptions): Client
    {
        this.options = { ...this.options, ...options };

        if (Array.isArray(options.peers))
        {
            this.handlePeers(options.peers)
        }
        if (options.persistStorage)
        {
            this.useConnector(
                new IndexedDbConnector(options.storageKey)
            );
        }
        if (Array.isArray(options.connectors))
        {
            options.connectors.forEach(connector => this.useConnector(connector));
        }

        return this;
    }

    /**
     * Traverse a location in the graph
     */
    get(soul: string): LexLink
    {
        return new LexLink(this, soul);
    }

    /**
     * System events Callback
     */
    on(event: string, cb: OnCb): Client
    {
        switch (event)
        {
            case 'auth':
                this._authEvent.on(cb);
                if (this._user?.is)
                {
                    this._authEvent.trigger(this._user.is);
                }
                break;
        }
        return this;
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
                this.graph['_opt'].mutable ? 'mutable' : 'immutable'
            )
        )
    }

    /**
     * Setup GraphConnector for graph
     */
    private useConnector(connector: GraphConnector): void
    {
        connector.sendPutsFromGraph(this.graph);
        connector.sendRequestsFromGraph(this.graph);
        this.graph.connect(connector);
    }

    /**
     * Connect to peers via connector SocketClusterConnector
     */
    private async handlePeers(peers: string[]): Promise<void>
    {
        peers.forEach((peer: string) =>
        {
            try
            {
                const url                                   = new URL(peer);
                const options: SocketClientOptions = {
                    hostname: url.hostname,
                    secure  : url.protocol.includes('https')
                };

                if (url.port.length > 0)
                {
                    options.port = Number(url.port);
                }

                this.useConnector(
                    createConnector(options)
                );
            }
            catch (e)
            {
                console.error(e);
            }
        });
    }
}
