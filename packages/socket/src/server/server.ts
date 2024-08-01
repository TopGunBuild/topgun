import { IncomingMessage, RequestObject, ServerChannelOptions, SocketServerOptions } from './types';
import {
    CodecEngine,
    EventObject,
    MIDDLEWARE_HANDSHAKE_TG,
    MIDDLEWARE_HANDSHAKE_WS,
    MIDDLEWARE_INVOKE, MIDDLEWARE_PUBLISH_IN, MIDDLEWARE_PUBLISH_OUT,
    MIDDLEWARE_SUBSCRIBE,
    MIDDLEWARE_TRANSMIT,
    MiddlewareFunction,
    Middlewares,
} from '../types';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { SimpleBroker, SimpleExchange } from '../simple-broker';
import { Socket } from './socket';
import { AsyncFunction, isNode, randomId } from '@topgunbuild/utils';
import {
    BrokerError,
    InvalidActionError,
    InvalidArgumentsError,
    InvalidOptionsError, ServerProtocolError, SilentMiddlewareBlockedError,
} from '../errors';
import { defaultCodecEngine } from '../default-codec-engine';
import { applyEachSeries } from '../utils/apply-each-series';

export class SocketServer extends AsyncStreamEmitter<any>
{
    options: SocketServerOptions;
    MIDDLEWARE_HANDSHAKE_WS: Middlewares;
    MIDDLEWARE_HANDSHAKE_TG: Middlewares;
    MIDDLEWARE_TRANSMIT: Middlewares;
    MIDDLEWARE_INVOKE: Middlewares;
    MIDDLEWARE_SUBSCRIBE: Middlewares;
    MIDDLEWARE_PUBLISH_IN: Middlewares;
    MIDDLEWARE_PUBLISH_OUT: Middlewares;

    origins: string;
    ackTimeout: number;
    handshakeTimeout: number;
    pingInterval: number;
    pingTimeout: number;
    pingTimeoutDisabled: boolean;
    allowClientPublish: boolean;
    perMessageDeflate: boolean|object;
    httpServer: any;
    socketChannelLimit: number;
    brokerEngine: SimpleBroker;
    appName: string;
    middlewareEmitWarnings: boolean;
    isReady: boolean;
    clients: {
        [id: string]: Socket
    };
    clientsCount: number;
    pendingClients: {
        [id: string]: Socket
    };
    pendingClientsCount: number;
    exchange: SimpleExchange;
    codec: CodecEngine;

    private readonly _middleware: { [key: string]: AsyncFunction[] };
    private readonly _allowAllOrigins: boolean;
    private readonly wsServer: any;
    private readonly _path: string;

    /**
     * Constructor
     */
    constructor(options: SocketServerOptions)
    {
        super();

        const opts: SocketServerOptions = {
            brokerEngine          : new SimpleBroker(),
            wsEngine              : 'ws',
            wsEngineServerOptions : {},
            allowClientPublish    : true,
            ackTimeout            : 10000,
            handshakeTimeout      : 10000,
            pingTimeout           : 20000,
            pingTimeoutDisabled   : false,
            pingInterval          : 8000,
            origins               : '*:*',
            appName               : randomId(),
            path                  : '/topgunsocket/',
            pubSubBatchDuration   : null,
            middlewareEmitWarnings: true,
        };

        this.options = Object.assign(opts, options || {});

        this.MIDDLEWARE_HANDSHAKE_WS = MIDDLEWARE_HANDSHAKE_WS;
        this.MIDDLEWARE_HANDSHAKE_TG = MIDDLEWARE_HANDSHAKE_TG;
        this.MIDDLEWARE_TRANSMIT     = MIDDLEWARE_TRANSMIT;
        this.MIDDLEWARE_INVOKE       = MIDDLEWARE_INVOKE;
        this.MIDDLEWARE_SUBSCRIBE    = MIDDLEWARE_SUBSCRIBE;
        this.MIDDLEWARE_PUBLISH_IN   = MIDDLEWARE_PUBLISH_IN;
        this.MIDDLEWARE_PUBLISH_OUT  = MIDDLEWARE_PUBLISH_OUT;

        this._middleware                               = {};
        this._middleware[this.MIDDLEWARE_HANDSHAKE_WS] = [];
        this._middleware[this.MIDDLEWARE_HANDSHAKE_TG] = [];
        this._middleware[this.MIDDLEWARE_TRANSMIT]     = [];
        this._middleware[this.MIDDLEWARE_INVOKE]       = [];
        this._middleware[this.MIDDLEWARE_SUBSCRIBE]    = [];
        this._middleware[this.MIDDLEWARE_PUBLISH_IN]   = [];
        this._middleware[this.MIDDLEWARE_PUBLISH_OUT]  = [];

        this.origins          = opts.origins;
        this._allowAllOrigins = this.origins.indexOf('*:*') !== -1;

        this.ackTimeout          = opts.ackTimeout;
        this.handshakeTimeout    = opts.handshakeTimeout;
        this.pingInterval        = opts.pingInterval;
        this.pingTimeout         = opts.pingTimeout;
        this.pingTimeoutDisabled = opts.pingTimeoutDisabled;
        this.allowClientPublish  = opts.allowClientPublish;
        this.httpServer          = opts.httpServer;
        this.socketChannelLimit  = opts.socketChannelLimit;

        this.brokerEngine           = opts.brokerEngine;
        this.appName                = opts.appName || '';
        this.middlewareEmitWarnings = opts.middlewareEmitWarnings;

        // Make sure there is always a leading and a trailing slash in the WS path.
        this._path = opts.path.replace(/\/?$/, '/').replace(/^\/?/, '/');

        if (this.brokerEngine.isReady)
        {
            this.isReady = true;
            this.emit('ready', {});
        }
        else
        {
            this.isReady = false;
            (async () =>
            {
                await this.brokerEngine.listener('ready').once();
                this.isReady = true;
                this.emit('ready', {});
            })();
        }

        if (opts.codecEngine)
        {
            this.codec = opts.codecEngine;
        }
        else
        {
            this.codec = defaultCodecEngine;
        }

        this.clients      = {};
        this.clientsCount = 0;

        this.pendingClients      = {};
        this.pendingClientsCount = 0;

        this.exchange = this.brokerEngine.exchange();

        const wsServerOptions        = opts.wsEngineServerOptions || {};
        wsServerOptions.server       = this.httpServer;
        wsServerOptions.verifyClient = this.verifyHandshake.bind(this);

        if (wsServerOptions.path == null && this._path != null)
        {
            wsServerOptions.path = this._path;
        }
        if (wsServerOptions.perMessageDeflate == null && this.perMessageDeflate != null)
        {
            wsServerOptions.perMessageDeflate = this.perMessageDeflate;
        }
        if (wsServerOptions.clientTracking == null)
        {
            wsServerOptions.clientTracking = false;
        }

        if (isNode())
        {
            const wsEngine = typeof opts.wsEngine === 'string' ? require(opts.wsEngine) : opts.wsEngine;
            if (!wsEngine || !wsEngine.Server)
            {
                throw new InvalidOptionsError(
                    'The wsEngine option must be a path or module name which points ' +
                    'to a valid WebSocket engine module with a compatible interface',
                );
            }
            const WSServer = wsEngine.Server;

            this.wsServer = new WSServer(wsServerOptions);
            this.wsServer.on('error', this._handleServerError.bind(this));
            this.wsServer.on('connection', this.handleSocketConnection.bind(this));
            // this.wsServer.on('connection', e => console.log('connection', e));
            // this.wsServer.on('error', e => console.log('error', e));
        }
    }

    handleSocketConnection(wsSocket: any, upgradeReq?: any): void
    {
        if (!wsSocket['upgradeReq'] && upgradeReq)
        {
            // Normalize ws modules to match.
            wsSocket['upgradeReq'] = upgradeReq;
        }

        const id = this.generateId();

        const tgSocket    = new Socket(id, this, wsSocket);
        tgSocket.exchange = this.exchange;

        this._handleSocketErrors(tgSocket);

        this.pendingClients[id] = tgSocket;
        this.pendingClientsCount++;

        const handleSocketSubscribe = async () =>
        {
            for await (const rpc of tgSocket.procedure('#subscribe'))
            {
                let channelOptions = rpc.data;

                if (!channelOptions)
                {
                    channelOptions = {};
                }
                else if (typeof channelOptions === 'string')
                {
                    channelOptions = {
                        channel: channelOptions,
                    };
                }

                (async () =>
                {
                    if (tgSocket.state === tgSocket.OPEN)
                    {
                        try
                        {
                            await this._subscribeSocket(tgSocket, channelOptions);
                        }
                        catch (err)
                        {
                            const error = new BrokerError(`Failed to subscribe socket to the ${channelOptions.channel} channel - ${err}`);
                            rpc.error(error);
                            tgSocket.emitError(error);
                            return;
                        }
                        if (channelOptions.batch)
                        {
                            rpc.end(undefined, { batch: true });
                            return;
                        }
                        rpc.end();
                        return;
                    }
                    // This is an invalid state; it means the client tried to subscribe before
                    // having completed the handshake.
                    const error = new InvalidActionError('Cannot subscribe socket to a channel before it has completed the handshake');
                    rpc.error(error);
                    this.emitWarning(error);
                })();
            }
        };
        handleSocketSubscribe();

        const handleSocketUnsubscribe = async () =>
        {
            for await (const rpc of tgSocket.procedure('#unsubscribe'))
            {
                const channel = rpc.data;
                let error;
                try
                {
                    this._unsubscribeSocket(tgSocket, channel);
                }
                catch (err)
                {
                    error = new BrokerError(
                        `Failed to unsubscribe socket from the ${channel} channel - ${err}`,
                    );
                }
                if (error)
                {
                    rpc.error(error);
                    tgSocket.emitError(error);
                }
                else
                {
                    rpc.end();
                }
            }
        };
        handleSocketUnsubscribe();

        const cleanupSocket = (type, code, reason) =>
        {
            clearTimeout(tgSocket._handshakeTimeoutRef);

            tgSocket.closeProcedure('#handshake');
            tgSocket.closeProcedure('#subscribe');
            tgSocket.closeProcedure('#unsubscribe');

            const isClientFullyConnected = !!this.clients[id];

            if (isClientFullyConnected)
            {
                delete this.clients[id];
                this.clientsCount--;
            }

            const isClientPending = !!this.pendingClients[id];
            if (isClientPending)
            {
                delete this.pendingClients[id];
                this.pendingClientsCount--;
            }

            if (type === 'disconnect')
            {
                this.emit('disconnection', {
                    socket: tgSocket,
                    code,
                    reason,
                });
            }
            else if (type === 'abort')
            {
                this.emit('connectionAbort', {
                    socket: tgSocket,
                    code,
                    reason,
                });
            }
            this.emit('closure', {
                socket: tgSocket,
                code,
                reason,
            });

            this._unsubscribeSocketFromAllChannels(tgSocket);
        };

        const handleSocketDisconnect = async () =>
        {
            const event = await tgSocket.listener('disconnect').once();
            cleanupSocket('disconnect', event.code, event.data);
        };
        handleSocketDisconnect();

        const handleSocketAbort = async () =>
        {
            const event = await tgSocket.listener('connectAbort').once();
            cleanupSocket('abort', event.code, event.data);
        };
        handleSocketAbort();

        tgSocket._handshakeTimeoutRef = setTimeout(this._handleHandshakeTimeout.bind(this, tgSocket), this.handshakeTimeout);

        const handleSocketHandshake = async () =>
        {
            for await (const rpc of tgSocket.procedure('#handshake'))
            {
                clearTimeout(tgSocket._handshakeTimeoutRef);

                this._passThroughHandshakeAGMiddleware({
                    socket: tgSocket,
                }, (err: any, statusCode: number) =>
                {
                    if (err)
                    {
                        if (err.statusCode == null)
                        {
                            err.statusCode = statusCode;
                        }
                        rpc.error(err);
                        tgSocket.disconnect(err.statusCode);
                        return;
                    }

                    if (tgSocket.state === tgSocket.CLOSED)
                    {
                        return;
                    }

                    const clientSocketStatus: EventObject = {
                        id         : tgSocket.id,
                        pingTimeout: this.pingTimeout
                    };
                    const serverSocketStatus: EventObject = {
                        id         : tgSocket.id,
                        pingTimeout: this.pingTimeout
                    };

                    if (this.pendingClients[id])
                    {
                        delete this.pendingClients[id];
                        this.pendingClientsCount--;
                    }
                    this.clients[id] = tgSocket;
                    this.clientsCount++;

                    tgSocket.state = tgSocket.OPEN;

                    tgSocket.emit('connect', serverSocketStatus);
                    this.emit('connection', { socket: tgSocket, ...serverSocketStatus });

                    rpc.end(clientSocketStatus);
                });
            }
        };
        handleSocketHandshake();

        // Emit event to signal that a socket handshake has been initiated.
        this.emit('handshake', { socket: tgSocket });
    }

    emitError(error: Error): void
    {
        this.emit('error', { error });
    }

    emitWarning(warning: Error): void
    {
        this.emit('warning', { warning });
    }

    close(keepSocketsOpen?: boolean): Promise<void>
    {
        this.isReady = false;
        return new Promise((resolve, reject) =>
        {
            if (isNode())
            {
                this.wsServer.close((err: any) =>
                {
                    if (err)
                    {
                        reject(err);
                        return;
                    }
                    resolve();
                });

                if (!keepSocketsOpen)
                {
                    for (const socket of Object.values(this.clients))
                    {
                        socket.terminate();
                    }
                }
            }
            else
            {
                for (const socket of Object.values(this.clients))
                {
                    socket.disconnect(1011, 'WebSocket broken.');
                }
            }
        });
    }

    getPath(): string
    {
        return this._path;
    };

    generateId(): string
    {
        return randomId();
    };

    addMiddleware(type: Middlewares, middleware: MiddlewareFunction): void
    {
        if (!this._middleware[type])
        {
            throw new InvalidArgumentsError(`Middleware type "${type}" is not supported`);
        }
        this._middleware[type].push(middleware);
    };

    removeMiddleware(type: Middlewares, middleware: MiddlewareFunction): void
    {
        const middlewareFunctions = this._middleware[type];

        this._middleware[type] = middlewareFunctions.filter((fn) =>
        {
            return fn !== middleware;
        });
    };

    async verifyHandshake(
        info: { origin?: string; secure?: boolean; req?: IncomingMessage },
        callback: (res: boolean, code?: number, message?: string, headers?: any) => void,
    ): Promise<void>
    {
        const req  = info.req;
        let origin = info.origin;
        if (origin === 'null' || origin == null)
        {
            origin = '*';
        }
        let ok: boolean|number = false;

        if (this._allowAllOrigins)
        {
            ok = true;
        }
        else
        {
            try
            {
                const parser = new URL(origin);
                const port   =
                          parser.port || (parser.protocol === 'https:' ? 443 : 80);
                ok           =
                    ~this.origins.indexOf(parser.hostname + ':' + port) ||
                    ~this.origins.indexOf(parser.hostname + ':*') ||
                    ~this.origins.indexOf('*:' + port);
            }
            catch (e)
            {
            }
        }

        if (ok)
        {
            const handshakeMiddleware = this._middleware[this.MIDDLEWARE_HANDSHAKE_WS];
            if (handshakeMiddleware.length)
            {
                let callbackInvoked = false;
                await applyEachSeries(handshakeMiddleware, req, (err) =>
                {
                    if (callbackInvoked)
                    {
                        this.emitWarning(
                            new InvalidActionError(
                                `Callback for ${this.MIDDLEWARE_HANDSHAKE_WS} middleware was already invoked`,
                            ),
                        );
                    }
                    else
                    {
                        callbackInvoked = true;
                        if (err)
                        {
                            if (err === true || err.silent)
                            {
                                err = new SilentMiddlewareBlockedError(
                                    `Action was silently blocked by ${this.MIDDLEWARE_HANDSHAKE_WS} middleware`,
                                    this.MIDDLEWARE_HANDSHAKE_WS,
                                );
                            }
                            else if (this.middlewareEmitWarnings)
                            {
                                this.emitWarning(err);
                            }
                            callback(false, 401, typeof err === 'string' ? err : err.message);
                        }
                        else
                        {
                            callback(true);
                        }
                    }
                });
            }
            else
            {
                callback(true);
            }
        }
        else
        {
            const err = new ServerProtocolError(
                `Failed to authorize socket handshake - Invalid origin: ${origin}`,
            );
            this.emitWarning(err);
            callback(false, 403, err.message);
        }
    }

    verifyInboundRemoteEvent(requestOptions: EventObject, callback: (err: Error, newEventData?: any, ackData?: any) => any): void
    {
        this._passThroughMiddleware(requestOptions, callback);
    }

    async verifyOutboundEvent(
        socket: any,
        eventName: string,
        eventData: any,
        options: any,
        callback: (err: Error, data?: any) => any,
    ): Promise<void>
    {
        let callbackInvoked = false;

        if (eventName === '#publish')
        {
            const request: EventObject = {
                socket : socket,
                channel: eventData.channel,
                data   : eventData.data,
            };
            await applyEachSeries(this._middleware[this.MIDDLEWARE_PUBLISH_OUT], request,
                (err) =>
                {
                    if (callbackInvoked)
                    {
                        this.emitWarning(
                            new InvalidActionError(
                                `Callback for ${this.MIDDLEWARE_PUBLISH_OUT} middleware was already invoked`,
                            ),
                        );
                    }
                    else
                    {
                        callbackInvoked = true;
                        if (request.data !== undefined)
                        {
                            eventData.data = request.data;
                        }
                        if (err)
                        {
                            if (err === true || err.silent)
                            {
                                err = new SilentMiddlewareBlockedError(
                                    `Action was silently blocked by ${this.MIDDLEWARE_PUBLISH_OUT} middleware`,
                                    this.MIDDLEWARE_PUBLISH_OUT,
                                );
                            }
                            else if (this.middlewareEmitWarnings)
                            {
                                this.emitWarning(err);
                            }
                            callback(err, eventData);
                        }
                        else
                        {
                            if (options && request.useCache)
                            {
                                options.useCache = true;
                            }
                            callback(null, eventData);
                        }
                    }
                },
            );
        }
        else
        {
            callback(null, eventData);
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async _processSubscribeAction(
        options: EventObject,
        request: RequestObject,
        callback: (err: Error, data?: any) => any,
    ): Promise<void>
    {
        let callbackInvoked = false;

        const eventData = options.data || {};
        request.channel = eventData.channel;
        request.data    = eventData.data;

        await applyEachSeries(this._middleware[this.MIDDLEWARE_SUBSCRIBE], request,
            (err) =>
            {
                if (callbackInvoked)
                {
                    this.emitWarning(
                        new InvalidActionError(
                            `Callback for ${this.MIDDLEWARE_SUBSCRIBE} middleware was already invoked`,
                        ),
                    );
                }
                else
                {
                    callbackInvoked = true;
                    if (err)
                    {
                        if (err === true || err.silent)
                        {
                            err = new SilentMiddlewareBlockedError(
                                `Action was silently blocked by ${this.MIDDLEWARE_SUBSCRIBE} middleware`,
                                this.MIDDLEWARE_SUBSCRIBE,
                            );
                        }
                        else if (this.middlewareEmitWarnings)
                        {
                            this.emitWarning(err);
                        }
                    }
                    if (request.data !== undefined)
                    {
                        eventData.data = request.data;
                    }
                    callback(err, eventData);
                }
            },
        );
    }

    private async _processTransmitAction(
        options: EventObject,
        request: RequestObject,
        callback: (err: Error, eventData: any, data?: any) => any,
    ): Promise<void>
    {
        let callbackInvoked = false;

        request.event = options.event;
        request.data  = options.data;

        await applyEachSeries(this._middleware[this.MIDDLEWARE_TRANSMIT], request,
            (err) =>
            {
                if (callbackInvoked)
                {
                    this.emitWarning(
                        new InvalidActionError(
                            `Callback for ${this.MIDDLEWARE_TRANSMIT} middleware was already invoked`,
                        ),
                    );
                }
                else
                {
                    callbackInvoked = true;
                    if (err)
                    {
                        if (err === true || err.silent)
                        {
                            err = new SilentMiddlewareBlockedError(
                                `Action was silently blocked by ${this.MIDDLEWARE_TRANSMIT} middleware`,
                                this.MIDDLEWARE_TRANSMIT,
                            );
                        }
                        else if (this.middlewareEmitWarnings)
                        {
                            this.emitWarning(err);
                        }
                    }
                    callback(err, request.data);
                }
            },
        );
    }

    private async _processPublishAction(
        options: EventObject,
        request: RequestObject,
        callback: (err: Error, eventData?: any, data?: any) => any,
    ): Promise<void>
    {
        let callbackInvoked = false;

        if (this.allowClientPublish)
        {
            const eventData = options.data || {};
            request.channel = eventData.channel;
            request.data    = eventData.data;

            await applyEachSeries(this._middleware[this.MIDDLEWARE_PUBLISH_IN], request,
                (err) =>
                {
                    if (callbackInvoked)
                    {
                        this.emitWarning(
                            new InvalidActionError(
                                `Callback for ${this.MIDDLEWARE_PUBLISH_IN} middleware was already invoked`,
                            ),
                        );
                    }
                    else
                    {
                        callbackInvoked = true;
                        if (request.data !== undefined)
                        {
                            eventData.data = request.data;
                        }
                        if (err)
                        {
                            if (err === true || err.silent)
                            {
                                err = new SilentMiddlewareBlockedError(
                                    `Action was silently blocked by ${this.MIDDLEWARE_PUBLISH_IN} middleware`,
                                    this.MIDDLEWARE_PUBLISH_IN,
                                );
                            }
                            else if (this.middlewareEmitWarnings)
                            {
                                this.emitWarning(err);
                            }
                            callback(err, eventData, request.ackData);
                        }
                        else
                        {
                            if (typeof request.channel !== 'string')
                            {
                                err = new BrokerError(
                                    `Socket ${request.socket.id} tried to publish to an invalid ${request.channel} channel`,
                                );
                                this.emitWarning(err);
                                callback(err, eventData, request.ackData);
                                return;
                            }
                            (async () =>
                            {
                                let error;
                                try
                                {
                                    await this.exchange.publish(request.channel, request.data);
                                }
                                catch (err)
                                {
                                    error = err;
                                    this.emitWarning(error);
                                }
                                callback(error, eventData, request.ackData);
                            })();
                        }
                    }
                },
            );
        }
        else
        {
            const noPublishError = new InvalidActionError('Client publish feature is disabled');
            this.emitWarning(noPublishError);
            callback(noPublishError);
        }
    }

    private async _processInvokeAction(
        options: EventObject,
        request: RequestObject,
        callback: (err: Error, data?: any) => any,
    ): Promise<void>
    {
        let callbackInvoked = false;

        request.event = options.event;
        request.data  = options.data;

        await applyEachSeries(this._middleware[this.MIDDLEWARE_INVOKE], request,
            (err) =>
            {
                if (callbackInvoked)
                {
                    this.emitWarning(
                        new InvalidActionError(
                            `Callback for ${this.MIDDLEWARE_INVOKE} middleware was already invoked`,
                        ),
                    );
                }
                else
                {
                    callbackInvoked = true;
                    if (err)
                    {
                        if (err === true || err.silent)
                        {
                            err = new SilentMiddlewareBlockedError(
                                `Action was silently blocked by ${this.MIDDLEWARE_INVOKE} middleware`,
                                this.MIDDLEWARE_INVOKE,
                            );
                        }
                        else if (this.middlewareEmitWarnings)
                        {
                            this.emitWarning(err);
                        }
                    }
                    callback(err, request.data);
                }
            },
        );
    }

    private _passThroughMiddleware(options: EventObject, callback: (err: Error, data?: any) => any): void
    {
        const request: RequestObject = {
            socket: options.socket,
        };

        const event = options.event;

        if (options.cid == null)
        {
            // If transmit.
            if (this._isReservedRemoteEvent(event))
            {
                if (event === '#publish')
                {
                    this._processPublishAction(options, request, callback);
                }
                else
                {
                    const error = new InvalidActionError(`The reserved transmitted event ${event} is not supported`);
                    callback(error);
                }
            }
            else
            {
                this._processTransmitAction(options, request, callback);
            }
        }
        else
        {
            // If invoke/RPC.
            if (this._isReservedRemoteEvent(event))
            {
                if (event === '#subscribe')
                {
                    this._processSubscribeAction(options, request, callback);
                }
                else if (event === '#publish')
                {
                    this._processPublishAction(options, request, callback);
                }
                else if (
                    event === '#handshake' ||
                    event === '#unsubscribe'
                )
                {
                    callback(null, options.data);
                }
                else
                {
                    const error = new InvalidActionError(`The reserved invoked event ${event} is not supported`);
                    callback(error);
                }
            }
            else
            {
                this._processInvokeAction(options, request, callback);
            }
        }
    }

    private _isReservedRemoteEvent(event?: string): boolean
    {
        return typeof event === 'string' && event.indexOf('#') === 0;
    }

    private _handleServerError(error: string|Error): void
    {
        if (typeof error === 'string')
        {
            error = new ServerProtocolError(error);
        }
        this.emitError(error);
    }

    private _handleHandshakeTimeout(scSocket: Socket): void
    {
        scSocket.disconnect(4005);
    }

    private async _handleSocketErrors(socket: any): Promise<void>
    {
        // A socket error will show up as a warning on the server.
        for await (const event of socket.listener('error'))
        {
            this.emitWarning(event.error);
        }
    }

    private async _subscribeSocket(socket: Socket, channelOptions: ServerChannelOptions): Promise<void>
    {
        if (!channelOptions)
        {
            throw new InvalidActionError(`Socket ${socket.id} provided a malformated channel payload`);
        }

        if (this.socketChannelLimit && socket.channelSubscriptionsCount >= this.socketChannelLimit)
        {
            throw new InvalidActionError(
                `Socket ${socket.id} tried to exceed the channel subscription limit of ${this.socketChannelLimit}`,
            );
        }

        const channelName = channelOptions.channel;

        if (typeof channelName !== 'string')
        {
            throw new InvalidActionError(`Socket ${socket.id} provided an invalid channel name`);
        }

        if (socket.channelSubscriptionsCount == null)
        {
            socket.channelSubscriptionsCount = 0;
        }
        if (socket.channelSubscriptions[channelName] == null)
        {
            socket.channelSubscriptions[channelName] = true;
            socket.channelSubscriptionsCount++;
        }

        try
        {
            await this.brokerEngine.subscribeSocket(socket, channelOptions);
        }
        catch (err)
        {
            delete socket.channelSubscriptions[channelName];
            socket.channelSubscriptionsCount--;
            throw err;
        }
        socket.emit('subscribe', {
            channel         : channelName,
            subscribeOptions: channelOptions,
        });
        this.emit('subscription', {
            socket,
            channel         : channelName,
            subscribeOptions: channelOptions,
        });
    }

    private _unsubscribeSocketFromAllChannels(socket: Socket): void
    {
        Object.keys(socket.channelSubscriptions).forEach((channelName) =>
        {
            this._unsubscribeSocket(socket, channelName);
        });
    };

    private _unsubscribeSocket(socket: Socket, channel: string): void
    {
        if (typeof channel !== 'string')
        {
            throw new InvalidActionError(
                `Socket ${socket.id} tried to unsubscribe from an invalid channel name`,
            );
        }
        if (!socket.channelSubscriptions[channel])
        {
            throw new InvalidActionError(
                `Socket ${socket.id} tried to unsubscribe from a channel which it is not subscribed to`,
            );
        }

        delete socket.channelSubscriptions[channel];
        if (socket.channelSubscriptionsCount != null)
        {
            socket.channelSubscriptionsCount--;
        }

        this.brokerEngine.unsubscribeSocket(socket, channel);

        socket.emit('unsubscribe', { channel });
        this.emit('unsubscription', { socket, channel });
    };

    private async _passThroughHandshakeAGMiddleware(
        options: EventObject,
        callback: (error?: Error, code?: number) => void,
    ): Promise<void>
    {
        let callbackInvoked = false;
        const request       = {
            socket: options.socket,
        };

        await applyEachSeries(this._middleware[this.MIDDLEWARE_HANDSHAKE_TG], request,
            (err, results) =>
            {
                if (callbackInvoked)
                {
                    this.emitWarning(
                        new InvalidActionError(
                            `Callback for ${this.MIDDLEWARE_HANDSHAKE_TG} middleware was already invoked`,
                        ),
                    );
                }
                else
                {
                    callbackInvoked = true;
                    let statusCode;
                    if (results.length)
                    {
                        statusCode = results[results.length - 1] || 4008;
                    }
                    else
                    {
                        statusCode = 4008;
                    }
                    if (err)
                    {
                        if (err.statusCode != null)
                        {
                            statusCode = err.statusCode;
                        }
                        if (err === true || err.silent)
                        {
                            err = new SilentMiddlewareBlockedError(
                                `Action was silently blocked by ${this.MIDDLEWARE_HANDSHAKE_TG} middleware`,
                                this.MIDDLEWARE_HANDSHAKE_TG,
                            );
                        }
                        else if (this.middlewareEmitWarnings)
                        {
                            this.emitWarning(err);
                        }
                    }
                    callback(err, statusCode);
                }
            },
        );
    }
}
