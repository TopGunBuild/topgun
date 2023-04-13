import { diffCRDT } from '../crdt';
import { TGLink } from './link';
import { TGGraph } from './graph/graph';
import { TGGraphConnector } from './transports/graph-connector';
import { DEFAULT_OPTIONS, TGClientOptions } from './client-options';
import { createConnector } from './transports/web-socket-graph-connector';
import { ClientOptions as SocketClientOptions } from 'topgun-socket/client';
import { TGUserApi } from './user-api';
import { pubFromSoul, unpackGraph } from '../sea';
import { polyfillGlobalThis } from '../utils/global-this';
import { isObject } from '../utils/is-object';
import { TGIndexedDbConnector } from '../indexeddb/indexeddb-connector';
import { TGOnCb, TGNode, TGUserReference } from '../types';
import { TGEvent } from './control-flow/event';
import { isString } from '../utils/is-string';
import { TGLexLink } from './lex-link';
import { match } from '../utils/match';

polyfillGlobalThis(); // Make "globalThis" available

export class TGClient 
{
    static match = match;

    options: TGClientOptions;
    pub: string | undefined;
    readonly graph: TGGraph;
    protected readonly _authEvent: TGEvent<TGUserReference>;
    protected _user?: TGUserApi;

    /**
     * Constructor
     */
    constructor(options?: TGClientOptions) 
    {
        options = isObject(options) ? options : {};
        this.options = { ...DEFAULT_OPTIONS, ...options };
        this._authEvent = new TGEvent<TGUserReference>('auth data');

        if (this.options && this.options.graph) 
        {
            this.graph = this.options.graph;
        }
        else 
        {
            this.graph = new TGGraph();
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

    user(): TGUserApi;
    user(pubOrNode: string | TGNode): TGLink;
    user(pubOrNode?: string | TGNode): TGUserApi | TGLink 
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

            return this.get('~' + this.pub);
        }

        return (this._user =
            this._user ||
            new TGUserApi(
                this,
                this.options.persistSession,
                this.options.sessionStorage,
                this.options.sessionStorageKey,
                this._authEvent,
            ));
    }

    opt(options: TGClientOptions): TGClient 
    {
        this.options = { ...this.options, ...options };

        if (Array.isArray(options.peers)) 
        {
            this.handlePeers(options.peers);
        }
        if (options.persistStorage) 
        {
            this.useConnector(new TGIndexedDbConnector(options.storageKey));
        }
        if (Array.isArray(options.connectors)) 
        {
            options.connectors.forEach(connector =>
                this.useConnector(connector),
            );
        }

        return this;
    }

    get(soul: string): TGLexLink 
    {
        return new TGLexLink(this, soul);
    }

    on(event: string, cb: TGOnCb): TGClient 
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

    private registerSeaMiddleware(): void 
    {
        this.graph.use(graph =>
            unpackGraph(
                graph,
                this.graph['_opt'].mutable ? 'mutable' : 'immutable',
            ),
        );
    }

    private useConnector(connector: TGGraphConnector): void 
    {
        connector.sendPutsFromGraph(this.graph);
        connector.sendRequestsFromGraph(this.graph);
        this.graph.connect(connector);
    }

    private async handlePeers(peers: string[]): Promise<void> 
    {
        peers.forEach((peer: string) => 
        {
            try 
            {
                const url = new URL(peer);
                const options: SocketClientOptions = {
                    hostname: url.hostname,
                    secure: url.protocol.includes('https'),
                };

                if (url.port.length > 0) 
                {
                    options.port = Number(url.port);
                }

                this.useConnector(createConnector(options));
            }
            catch (e) 
            {
                console.error(e);
            }
        });
    }
}
