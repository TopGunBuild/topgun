import { Struct, Result, ok, isErr, isFunction, isObject, isDefined } from '@topgunbuild/typed';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { pseudoRandomText, verify } from '../sea';
import { TGGraphAdapter, TGGraphData, TGMessage } from '../types';
import { TGServerOptions } from './server-options';
import { listen, TGSocketServer, TGSocket } from '@topgunbuild/socket/server';
import { createMemoryAdapter } from '../memory-adapter';
import { createValidator } from '../validator';
import { Middleware } from './middleware';
import { uuidv4 } from '../utils/uuidv4';
import { MAX_KEY_SIZE, MAX_VALUE_SIZE } from '../storage';
import { TGFederationAdapter } from '../federation-adapter/federation-adapter';
import { TGPeers } from '../federation-adapter/peers';
import { createLogger, TGExtendedLoggerType, TGLoggerType } from '../logger';

export class TGServer extends AsyncStreamEmitter<any>
{
    readonly adapter: TGFederationAdapter;
    readonly internalAdapter: TGGraphAdapter;
    readonly gateway: TGSocketServer;
    readonly options: TGServerOptions;
    readonly middleware: Middleware;
    readonly peers: TGPeers;
    readonly validator: Struct<TGGraphData>;

    private logger: TGLoggerType;
    private pruneInterval: any;
    private peersDisconnector: () => void;

    /**
     * Constructor
     */
    constructor(options?: TGServerOptions)
    {
        super();
        const defaultOptions: TGServerOptions = {
            maxKeySize            : MAX_KEY_SIZE,
            maxValueSize          : MAX_VALUE_SIZE,
            disableValidation     : false,
            authMaxDrift          : 1000 * 60 * 5,
            peerSyncInterval      : 1000,
            peerPruneInterval     : 60 * 60 * 1000,
            peerBackSync          : 0,
            peerChangelogRetention: 0,
            peers                 : [],
        };

        this.options = Object.assign(defaultOptions, options || {});

        this.#createLogger();

        this.validator       = createValidator();
        this.internalAdapter = this.options.adapter || createMemoryAdapter(options);
        this.peers           = new TGPeers(this.options.peers);
        this.adapter         = this.#federateInternalAdapter(this.internalAdapter);
        this.gateway         = listen(this.options.port, this.options);
        this.middleware      = new Middleware(this.gateway, this.options, this.adapter);
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
        clearInterval(this.pruneInterval);
        this.peersDisconnector();
        this.peers.forEach(value => value.close());
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
        this.#prune();
        this.pruneInterval     = setInterval(this.#prune.bind(this), this.options.peerPruneInterval);
        this.peersDisconnector = this.adapter.connectToPeers();

        this.logger.log('TopGun application successfully started');
    }

    async #prune(): Promise<void>
    {
        const before = new Date().getTime() - (this.options.peerChangelogRetention || 0);
        return (
            this.internalAdapter.pruneChangelog &&
            this.internalAdapter.pruneChangelog(before)
        )
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
     * Wrap internal adapter
     */
    #federateInternalAdapter(adapter: TGGraphAdapter): TGFederationAdapter
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

        const withValidation = {
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

        return new TGFederationAdapter(
            withPublish,
            this.peers,
            withValidation,
            {
                backSync     : this.options.peerBackSync,
                batchInterval: this.options.peerBatchInterval,
                maxStaleness : this.options.peerMaxStaleness,
                putToPeers   : true
            }
        )
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
                // const url = socket.request.headers.host + socket.request.url;
                //
                // if (this.peerSet[url])
                // {
                //     const err = new Error('Peer already connect');
                //     err.name = 'FailedPeerConnect';
                //     socket.disconnect();
                // }

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

    #createLogger(): void
    {
        if (!isObject(this.options.log))
        {
            this.options.log = {};
        }
        if (!isDefined(this.options.log.appId) && this.options.port)
        {
            this.options.log.appId = this.options.port;
        }
        this.logger = createLogger(this.options.log);
    }
}

