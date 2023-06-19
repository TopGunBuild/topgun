import { isObject, isString, isFunction, enums, unwrap } from 'topgun-typed';
import { diffCRDT } from '../crdt';
import { TGLink } from './link';
import { TGGraph } from './graph/graph';
import { TGGraphConnector } from './transports/graph-connector';
import { DEFAULT_OPTIONS, TGClientOptions, TGClientPeerOptions } from './client-options';
import { createConnector, TGWebSocketGraphConnector } from './transports/web-socket-graph-connector';
import { TGSocketClientOptions } from 'topgun-socket/client';
import { TGUserApi } from './user-api';
import { pubFromSoul, unpackGraph } from '../sea';
import { TGIndexedDBConnector } from '../indexeddb/indexeddb-connector';
import { TGNode, TGUserReference, SystemEvent } from '../types';
import { TGEvent } from './control-flow/event';
import { TGLexLink } from './lex-link';
import { match } from '../utils/match';
import { wait } from '../utils/wait';
import { assertNotEmptyString, assertObject } from '../utils/assert';

/**
 * Main entry point for TopGun in browser
 */
export class TGClient
{
    static match = match;

    options: TGClientOptions;
    pub: string|undefined;
    readonly graph: TGGraph;
    protected readonly _authEvent: TGEvent<TGUserReference>;
    protected _user?: TGUserApi;
    readonly WAIT_FOR_USER_PUB: string;

    /**
     * Constructor
     */
    constructor(options?: TGClientOptions)
    {
        options                = isObject(options) ? options : {};
        this.options           = { ...DEFAULT_OPTIONS, ...options };
        this._authEvent        = new TGEvent<TGUserReference>('auth data');
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
                this._authEvent,
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
    get(soul: string): TGLexLink
    {
        return new TGLexLink(this, assertNotEmptyString(soul));
    }

    /**
     * System events callback
     */
    on(event: SystemEvent, cb: (value) => void, once = false): TGClient
    {
        const struct = enums(SystemEvent);
        const actual = struct(event);

        switch (unwrap(actual))
        {
        case 'auth':
            const _cb = (value) =>
            {
                if (isFunction(cb))
                {
                    cb(value);
                }
                if (once)
                {
                    this._authEvent.off(_cb);
                }
            };
            this._authEvent.on(_cb);

            if (this._user?.is)
            {
                // Execute immediately if the user is authorized
                this._authEvent.trigger(this._user.is);
            }
            break;
        }
        return this;
    }

    /**
     * Return system event as promise
     */
    promise<T>(event: SystemEvent): Promise<T>
    {
        return new Promise<T>(resolve => this.on(event, resolve, true));
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
            if (connector instanceof TGWebSocketGraphConnector)
            {
                const cleanupTasks = [];
                const client       = connector.client;

                if (client)
                {
                    if (client.state !== client.CLOSED)
                    {
                        cleanupTasks.push(
                            Promise.race([
                                client.listener('disconnect').once(),
                                client.listener('connectAbort').once()
                            ])
                        );
                        client.disconnect();
                    }
                    else
                    {
                        client.disconnect();
                    }
                }

                await Promise.all(cleanupTasks);
            }
            else
            {
                connector.disconnect();
            }
        });
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

export function createClient(options?: TGClientOptions): TGClient
{
    return new TGClient(options);
}