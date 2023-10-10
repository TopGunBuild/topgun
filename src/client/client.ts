import { isObject, isString } from '@topgunbuild/typed';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { diffCRDT } from '../crdt';
import { TGLink } from './link/link';
import { TGGraph } from './graph/graph';
import { TGGraphConnector } from './transports/graph-connector';
import { TGClientOptions } from './client-options';
import { createConnector } from './transports/web-socket-graph-connector';
import { TGUserApi } from './user-api';
import { pubFromSoul, unpackGraph } from '../sea';
import { TGIndexedDBConnector } from '../indexeddb/indexeddb-connector';
import { TGPeerOptions, TGNode } from '../types';
import { match } from '../utils/match';
import { assertObject, assertGetPath } from '../utils/assert';
import { getSessionStorage, getSessionStorageKey, localStorageAdapter } from '../utils';
import { socketOptionsFromPeer } from '../utils/socket-options-from-peer';
import { MAX_KEY_SIZE, MAX_VALUE_SIZE } from '../storage';

/**
 * Main entry point for TopGun in browser
 */
export class TGClient extends AsyncStreamEmitter<any>
{
    static match = match;

    pub: string|undefined;
    options: TGClientOptions;

    readonly graph: TGGraph;
    protected _user?: TGUserApi;
    readonly WAIT_FOR_USER_PUB: string;

    /**
     * Constructor
     */
    constructor(options?: TGClientOptions)
    {
        super();
        const defaultOptions: TGClientOptions = {
            peers                    : [],
            connectors               : [],
            localStorage             : false,
            localStorageKey          : 'topgun-nodes',
            sessionStorage           : localStorageAdapter,
            sessionStorageKey        : 'topgun-session',
            passwordMinLength        : 8,
            passwordMaxLength        : 48,
            transportMaxKeyValuePairs: 200,
            maxKeySize               : MAX_KEY_SIZE,
            maxValueSize             : MAX_VALUE_SIZE
        };

        this.graph             = new TGGraph(this);
        this.WAIT_FOR_USER_PUB = '__WAIT_FOR_USER_PUB__';

        this.graph.use(diffCRDT);
        this.graph.use(diffCRDT, 'write');

        this.opt(Object.assign(defaultOptions, options || {}));
        this.#registerSeaMiddleware();
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
                getSessionStorage(this.options?.sessionStorage),
                getSessionStorageKey(this.options?.sessionStorageKey),
            ));
    }

    /**
     * Set TopGun configuration options
     */
    opt(options: TGClientOptions): TGClient
    {
        this.options = assertObject(options);

        if (Array.isArray(this.options.peers))
        {
            this.#handlePeers(this.options.peers);
        }
        if (this.options.localStorage)
        {
            this.#useConnector(new TGIndexedDBConnector(this.options.localStorageKey, this.options));
        }
        if (Array.isArray(this.options.connectors))
        {
            this.options.connectors.forEach(connector =>
                this.#useConnector(connector),
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
        await this.graph.eachConnector(async (connector) =>
        {
            await connector.disconnect();
        });
        this.closeAllListeners();
    }

    async waitForConnect<T extends TGGraphConnector>(): Promise<T>
    {
        return await this.listener('connectorConnected').once();
    }

    /**
     * System events callback
     */
    on(event: string, cb: (value) => void): void
    {
        (async () =>
        {
            for await (const value of this.listener(event))
            {
                cb(value);
            }
        })();
    }

    connectors(): TGGraphConnector[]
    {
        return this.graph.connectors;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Register middleware with Security, Encryption, & Authorization - SEA
     */
    #registerSeaMiddleware(): void
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
    #useConnector(connector: TGGraphConnector): void
    {
        connector.sendPutsFromGraph(this.graph);
        connector.sendRequestsFromGraph(this.graph);
        this.graph.connect(connector);
    }

    /**
     * Connect to peers via connector TopGunSocket
     */
    async #handlePeers(peers: TGPeerOptions[]): Promise<void>
    {
        peers.forEach((peer: TGPeerOptions) =>
        {
            try
            {
                const socketOpts = socketOptionsFromPeer(peer);

                if (socketOpts)
                {
                    this.#useConnector(createConnector(socketOpts));
                }
            }
            catch (e)
            {
                console.error(e);
            }
        });
    }
}
