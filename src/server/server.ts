import { Struct, Result, ok, isErr, isObject } from 'topgun-typed';
import { InboundMiddleware } from './middlewares/inbound-middleware';
import { pseudoRandomText, verify } from '../sea';
import { TGGraphAdapter, TGGraphData, TGMessage } from '../types';
import { MiddlewareInboundStrategy } from './middlewares/strategy/middleware-inbound-strategy';
import { TGServerOptions } from './server-options';
import {
    MIDDLEWARE_INBOUND,
    TGActionAuthenticate,
    TGActionInvoke,
    TGActionPublishIn,
    TGActionSubscribe,
    TGActionTransmit,
    TGServerSocketGateway,
    TGServerSocket,
    listen,
} from 'topgun-socket/server';
import { WritableConsumableStream } from 'topgun-socket/writable-consumable-stream';
import { createMemoryAdapter } from '../memory-adapter';
import { generateMessageId } from '../client/graph/graph-utils';
import { createValidator } from '../validator';

export class TGServer 
{
    readonly adapter: TGGraphAdapter;
    readonly internalAdapter: TGGraphAdapter;
    readonly server: TGServerSocketGateway;
    readonly options: TGServerOptions;

    protected readonly validator: Struct<TGGraphData>;

    /**
     * Constructor
     */
    constructor(options: TGServerOptions) 
    {
        this.options = isObject(options) ? options : {};
        this.validator = createValidator();
        this.internalAdapter = this.options.adapter || createMemoryAdapter();
        this.adapter = this.wrapAdapter(this.internalAdapter);
        this.server = listen(this.options.port, this.options);
        this.run();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Protected methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Run server
     */
    protected run(): void 
    {
        this.setInboundMiddleware(
            new InboundMiddleware(this.adapter, this.options),
        );
        this.handleWebsocketConnection();
    }

    /**
     * Send put data to all node subscribers
     */
    protected publishDiff(
        soul: string,
        msgId: string,
        nodeDiff: TGGraphData,
    ): void 
    {
        this.server.exchange.invokePublish(`topgun/nodes/${soul}`, {
            '#': `${msgId}/${soul}`,
            'put': {
                [soul]: nodeDiff,
            },
        });
    }

    /**
     * Wrap adapter
     */
    protected wrapAdapter(adapter: TGGraphAdapter): TGGraphAdapter 
    {
        const withPublish: TGGraphAdapter = {
            ...adapter,
            put: async (graph: TGGraphData) => 
            {
                const diff = await adapter.put(graph);

                if (diff) 
                {
                    this.publishIsDiff({
                        '#': pseudoRandomText(),
                        'put': diff,
                    });
                }

                return diff;
            },
            putSync: undefined,
        };

        return {
            ...withPublish,
            put: async (graph: TGGraphData) => 
            {
                const result = this.validatePut(graph);

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
    protected publishIsDiff(msg: TGMessage): void 
    {
        const msgId = msg['#'] || generateMessageId();
        const diff = msg.put;

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

            this.publishDiff(soul, msgId, nodeDiff);
        }
    }

    /**
     * Validate put operation
     */
    protected validatePut(graph: TGGraphData): Result<TGGraphData> 
    {
        if (this.options.disableValidation) 
        {
            return ok(graph);
        }
        return this.validator(graph);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Set up a loop to handle websocket connections.
     */
    private async handleWebsocketConnection(): Promise<void> 
    {
        for await (const { socket } of this.server.listener('connection')) 
        {
            (async () => 
            {
                // Set up a loop to handle and respond to RPCs.
                for await (const request of socket.procedure('login')) 
                {
                    this.authenticateLogin(socket, request);
                }
            })();
        }
    }

    /**
     * Authenticate a connection for extra privileges
     */
    private async authenticateLogin(
        socket: TGServerSocket,
        request: any,
    ): Promise<void> 
    {
        const data = request.data as {
            pub: string;
            proof: {
                m: string;
                s: string;
            };
        };

        if (!data.pub || !data.proof) 
        {
            request.end('Missing login info');
            return;
        }

        try 
        {
            const [socketId, timestampStr] = data.proof.m.split('/');
            const timestamp = parseInt(timestampStr, 10);
            const now = new Date().getTime();
            const drift = Math.abs(now - timestamp);
            const maxDrift =
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
                socket.setAuthToken({
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

    /**
     * Setup a MIDDLEWARE_INBOUND
     */
    private setInboundMiddleware(inbound: MiddlewareInboundStrategy) 
    {
        if (inbound) 
        {
            // debug.log('Middleware inbound -> ' + inbound.constructor.name);

            this.server.setMiddleware(
                MIDDLEWARE_INBOUND,
                async (middlewareStream: WritableConsumableStream<any>) => 
                {
                    for await (const action of middlewareStream) 
                    {
                        this.handleInboundAction(action, inbound);
                    }
                },
            );
        }
    }

    /**
     * Handling MIDDLEWARE_INBOUND actions
     */
    private handleInboundAction(
        action:
            | TGActionTransmit
            | TGActionInvoke
            | TGActionSubscribe
            | TGActionPublishIn
            | TGActionAuthenticate,
        inbound: MiddlewareInboundStrategy,
    ) 
    {
        switch (action.type) 
        {
        case action.AUTHENTICATE:
            inbound.onAuthenticate
                ? inbound.onAuthenticate(action)
                : inbound.default(action);
            break;
        case action.SUBSCRIBE:
            inbound.onSubscribe
                ? inbound.onSubscribe(action)
                : inbound.default(action);
            break;
        case action.TRANSMIT:
            inbound.onTransmit
                ? inbound.onTransmit(action)
                : inbound.default;
            break;
        case action.INVOKE:
            inbound.onInvoke
                ? inbound.onInvoke(action)
                : inbound.default(action);
            break;
        case action.PUBLISH_IN:
            inbound.onPublishIn
                ? inbound.onPublishIn(action)
                : inbound.default(action);
            break;
        default:
            console.warn(`Not implemented type "${action}"!`);
            inbound.default(action);
        }
    }
}

export function createServer(serverConfig: TGServerOptions): TGServer 
{
    return new TGServer(serverConfig);
}
