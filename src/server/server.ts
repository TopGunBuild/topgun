import { Struct, Result, ok, isErr, isFunction, isObject, isDefined, isString } from '@topgunbuild/typed';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { pseudoRandomText } from '../sea';
import { TGGraphAdapter, TGGraphData, TGMessage, TGOriginators } from '../types';
import { TGServerOptions } from './server-options';
import { listen, TGSocketServer, TGSocket } from '@topgunbuild/socket/server';
import { createMemoryAdapter } from '../memory-adapter';
import { createValidator } from '../validator';
import { Middleware } from './middleware';
import { uuidv4 } from '../utils/uuidv4';
import { MAX_KEY_SIZE, MAX_VALUE_SIZE } from '../storage';
import { TGFederationAdapter } from '../federation-adapter/federation-adapter';
import { TGPeers } from '../federation-adapter/peers';
import { createLogger, TGLoggerType } from '../logger';
import { socketLoginHandler } from './utils/socket-login-handler';

export class TGServer extends AsyncStreamEmitter<any>
{
    appName: string;
    readonly adapter: TGFederationAdapter;
    readonly internalAdapter: TGGraphAdapter;
    readonly gateway: TGSocketServer;
    readonly options: TGServerOptions;
    readonly middleware: Middleware;
    readonly peers: TGPeers;
    readonly validator: Struct<TGGraphData>;

    private logger: TGLoggerType;
    private peersDisconnector: () => void;

    /**
     * Constructor
     */
    constructor(options?: TGServerOptions)
    {
        super();
        const defaultOptions: TGServerOptions = {
            maxKeySize       : MAX_KEY_SIZE,
            maxValueSize     : MAX_VALUE_SIZE,
            disableValidation: false,
            peers            : [],
            putToPeers       : true,
            reversePeerSync  : true
        };

        this.options = Object.assign(defaultOptions, options || {});
        this.gateway = listen(this.options.port, this.options);

        this.#persistAppName();
        this.#createLogger();

        this.validator       = createValidator();
        this.internalAdapter = this.options.adapter || createMemoryAdapter(options);
        this.peers           = new TGPeers(this.options.peers);
        this.adapter         = this.#federateInternalAdapter(this.internalAdapter);
        this.middleware      = new Middleware(this.gateway, this.options, this.adapter);
        this.#run();

        console.log(this.gateway.httpServer.address())
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
        this.peersDisconnector();
        this.peers.forEach(peer => peer.disconnect());
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
        this.peersDisconnector = this.adapter.connectToPeers();
        this.logger.log('TopGun application successfully started');
    }

    /**
     * Wrap internal adapter
     */
    #federateInternalAdapter(adapter: TGGraphAdapter): TGFederationAdapter
    {
        const withPublish: TGGraphAdapter = {
            ...adapter,
            put: async (graph: TGGraphData, originators?: TGOriginators) =>
            {
                const diff = await adapter.put(graph);

                if (diff)
                {
                    this.#publishIsDiff({
                        '#'  : pseudoRandomText(),
                        'put': diff,
                        originators
                    });
                }

                return diff;
            },
        };

        const withValidation = {
            ...withPublish,
            put: async (graph: TGGraphData, originators?: TGOriginators) =>
            {
                const result = this.#validatePut(graph);

                if (isErr(result))
                {
                    throw result.error;
                }

                return withPublish.put(graph, originators);
            },
        };

        return new TGFederationAdapter(
            withPublish,
            this.peers,
            withValidation,
            {
                putToPeers     : this.options.putToPeers,
                reversePeerSync: this.options.reversePeerSync
            },
            this.logger.extend('FederationAdapter')
        )
    }

    /**
     * Send put data to node subscribers as a diff
     */
    #publishIsDiff(msg: TGMessage): void
    {
        const msgId       = msg['#'] || uuidv4();
        const diff        = msg.put;
        const originators = msg.originators;

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

            this.#publishDiff(soul, msgId, nodeDiff, originators);
        }
    }

    /**
     * Send put data to all node subscribers
     */
    #publishDiff(
        soul: string,
        msgId: string,
        nodeDiff: TGGraphData,
        originators?: Record<string, number>
    ): void
    {
        this.gateway.exchange.publish(`topgun/nodes/${soul}`, {
            '#'  : `${msgId}/${soul}`,
            'put': {
                [soul]: nodeDiff,
            },
            'originators': {
                ...(originators || {}),
                [this.appName]: 1
            }
        });
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
            this.#loginProcedureListener(socket);
        }
    }

    /**
     * RPC listener for a socket's login
     */
    async #loginProcedureListener(socket: TGSocket): Promise<void>
    {
        for await (const request of socket.procedure('login'))
        {
            socketLoginHandler(socket, request);
        }
    }

    #createLogger(): void
    {
        if (!isObject(this.options.log))
        {
            this.options.log = {};
        }
        if (!isDefined(this.options.log.appId) && isString(this.appName))
        {
            this.options.log.appId = this.appName;
        }
        this.logger = createLogger(this.options.log);
    }

    #persistAppName(): void
    {
        this.appName = this.options.appName;

        if (!isString(this.options.appName))
        {
            if (isFunction(this.gateway.httpServer?.address))
            {
                const address = this.gateway.httpServer?.address();
                this.appName  = address.address + address.port;
            }
            else if (this.options.port)
            {
                this.appName = String(this.options.port);
            }
        }
    }
}

