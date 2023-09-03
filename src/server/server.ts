import { Struct, Result, ok, isErr, isObject, isFunction } from '@topgunbuild/typed';
import { pseudoRandomText, verify } from '../sea';
import { TGGraphAdapter, TGGraphData, TGMessage, TGPeerOptions } from '../types';
import { TGServerOptions } from './server-options';
import { listen, TGSocketServer, TGSocket } from '@topgunbuild/socket/server';
import { createMemoryAdapter } from '../memory-adapter';
import { createValidator } from '../validator';
import { Middleware } from './middleware';
import { uuidv4 } from '../utils/uuidv4';
import { socketOptionsFromPeer } from '../utils/socket-options-from-peer';
import { createConnector, TGWebSocketGraphConnector } from '../client/transports/web-socket-graph-connector';

export class TGServer
{
    readonly adapter: TGGraphAdapter;
    readonly internalAdapter: TGGraphAdapter;
    readonly gateway: TGSocketServer;
    readonly options: TGServerOptions;
    readonly middleware: Middleware;
    readonly peerConnectors: Map<string, TGWebSocketGraphConnector>;

    protected readonly validator: Struct<TGGraphData>;

    /**
     * Constructor
     */
    constructor(options?: TGServerOptions)
    {
        this.options         = isObject(options) ? options : {};
        this.validator       = createValidator();
        this.internalAdapter = this.options.adapter || createMemoryAdapter(options);
        this.adapter         = this.#wrapAdapter(this.internalAdapter);
        this.gateway         = listen(this.options.port, this.options);
        this.middleware      = new Middleware(this.gateway, this.options, this.adapter);
        this.peerConnectors  = new Map<string, TGWebSocketGraphConnector>();
        this.#run();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async waitForReady(): Promise<void>
    {
        await this.gateway.listener('ready').once();
    }

    async close(): Promise<void>
    {
        if (isFunction(this.gateway.httpServer?.close))
        {
            this.gateway.httpServer.close();
        }
        await this.gateway.close();
    }

    /**
     * Invoked to create a direct connection to another tg server & publish or subscribe
     */
    async connectToServer(peer: TGPeerOptions): Promise<void>
    {
        try
        {
            const opts      = socketOptionsFromPeer(peer);
            const connector = createConnector(opts);
            const uri       = connector.client.transport.uri();

            if (this.peerConnectors.has(uri))
            {
                this.peerConnectors.get(uri).disconnect();
            }

            this.peerConnectors.set(uri, connector);
        }
        catch (e)
        {
            console.error(e);
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Run server
     */
    #run(): void
    {
        this.middleware.setupMiddleware();
        this.#handleWebsocketConnection();
        if (Array.isArray(this.options?.peers))
        {
            this.options.peers.forEach(peer => this.connectToServer(peer));
        }
    }

    /**
     * Send put data to all node subscribers
     */
    #publishDiff(
        soul: string,
        msgId: string,
        nodeDiff: TGGraphData,
    ): void
    {
        this.gateway.exchange.publish(`topgun/nodes/${soul}`, {
            '#'  : `${msgId}/${soul}`,
            'put': {
                [soul]: nodeDiff,
            },
        });
    }

    /**
     * Wrap adapter
     */
    #wrapAdapter(adapter: TGGraphAdapter): TGGraphAdapter
    {
        const withPublish: TGGraphAdapter = {
            ...adapter,
            put: async (graph: TGGraphData) =>
            {
                const diff = await adapter.put(graph);

                if (diff)
                {
                    this.#publishIsDiff({
                        '#'  : pseudoRandomText(),
                        'put': diff,
                    });
                }

                return diff;
            },
        };

        return {
            ...withPublish,
            put: async (graph: TGGraphData) =>
            {
                const result = this.#validatePut(graph);

                if (isErr(result))
                {
                    throw result.error;
                }

                return withPublish.put(graph);
            },
        };
    }

    /**
     * Send put data to node subscribers as a diff
     */
    #publishIsDiff(msg: TGMessage): void
    {
        const msgId = msg['#'] || uuidv4();
        const diff  = msg.put;

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

            const nodeDiff = diff[soul];

            if (!nodeDiff)
            {
                continue;
            }

            this.#publishDiff(soul, msgId, nodeDiff);
        }
    }

    /**
     * Validate put operation
     */
    #validatePut(graph: TGGraphData): Result<TGGraphData>
    {
        if (this.options.disableValidation)
        {
            return ok(graph);
        }
        return this.validator(graph);
    }

    /**
     * Set up a loop to handle websocket connections.
     */
    async #handleWebsocketConnection(): Promise<void>
    {
        for await (const { socket } of this.gateway.listener('connection'))
        {
            (async () =>
            {
                // Set up a loop to handle and respond to RPCs.
                for await (const request of (socket as TGSocket).procedure('login'))
                {
                    this.#authenticateLogin(socket, request);
                }
            })();
        }
    }

    /**
     * Authenticate a connection for extra privileges
     */
    async #authenticateLogin(
        socket: TGSocket,
        request: {
            data: {
                pub: string;
                proof: {
                    m: string;
                    s: string;
                };
            },
            end: (reason?: string) => void,
            error: (error?: Error) => void
        },
    ): Promise<void>
    {
        const data = request.data;

        if (!data.pub || !data.proof)
        {
            request.end('Missing login info');
            return;
        }

        try
        {
            const [socketId, timestampStr] = data.proof.m.split('/');
            const timestamp                = parseInt(timestampStr, 10);
            const now                      = new Date().getTime();
            const drift                    = Math.abs(now - timestamp);
            const maxDrift                 =
                      (this.options.authMaxDrift &&
                          parseInt(`${this.options.authMaxDrift}`, 10)) ||
                      1000 * 60 * 5;

            if (drift > maxDrift)
            {
                request.error(new Error('Exceeded max clock drift'));
                return;
            }

            if (!socketId || socketId !== socket.id)
            {
                request.error(new Error('Socket ID doesn\'t match'));
                return;
            }

            const isVerified = await verify(data.proof, data.pub);

            if (isVerified)
            {
                await socket.setAuthToken({
                    pub: data.pub,
                    timestamp,
                });
                request.end();
            }
            else
            {
                request.end('Invalid login');
            }
        }
        catch (err)
        {
            request.end('Invalid login');
        }
    }
}

