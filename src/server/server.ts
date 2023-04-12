import { InboundMiddleware } from './middlewares/inbound-middleware';
import { pseudoRandomText, verify } from '../sea';
import { TGGraphAdapter, TGGraphData, TGMessage } from '../types';
import { MiddlewareInboundStrategy } from './middlewares/strategy/middleware-inbound-strategy';
import { TGServerOptions } from './server-options';
import { createValidator } from '../validator-sea';
import { ValidateFunction } from 'ajv';
import {
    MIDDLEWARE_INBOUND,
    TGActionAuthenticate,
    TGActionInvoke,
    TGActionPublishIn,
    TGActionSubscribe,
    TGActionTransmit,
    TGServerSocketGateway,
    TGServerSocket,
} from 'topgun-socket/server';
import { WritableConsumableStream } from 'topgun-socket/writable-consumable-stream';
import { createMemoryAdapter } from '../memory-adapter';
import { generateMessageId } from '../client/graph/graph-utils';

export class TGServer 
{
    public readonly adapter: TGGraphAdapter;
    public readonly internalAdapter: TGGraphAdapter;
    public readonly server: TGServerSocketGateway;

    protected readonly validaror: {
        schema: any;
        validate: ValidateFunction<any>;
    };

    /**
     * Constructor
     */
    constructor(public readonly options: TGServerOptions) 
{
        this.validaror = createValidator();
        this.internalAdapter = this.options.adapter || createMemoryAdapter();
        this.adapter = this.wrapAdapter(this.internalAdapter);
        this.server = new TGServerSocketGateway(this.options);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Run server
     */
    run(): void 
{
        this.setInboundMiddleware(
            new InboundMiddleware(this.adapter, this.options),
        );
        this.handleWebsocketConnection();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Protected methods
    // -----------------------------------------------------------------------------------------------------

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
            put: (graph: TGGraphData) => 
{
                return this.validatePut(graph).then((isValid) => 
{
                    if (isValid) 
{
                        return withPublish.put(graph);
                    }

                    throw new Error('Invalid graph data');
                });
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
    protected async validatePut(graph: TGGraphData): Promise<boolean> 
{
        if (this.options.disableValidation) 
{
            return true;
        }
        return this.validaror.validate({
            '#': 'dummymsgid',
            'put': graph,
        });
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