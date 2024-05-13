import { isNode } from '@topgunbuild/utils';
import {
    hydrateError,
    InvalidArgumentsError, SocketProtocolError,
    socketProtocolErrorStatuses,
    socketProtocolIgnoreStatuses, TimeoutError,
} from '../errors/errors';
import { Response } from '../response';
import { AsyncStreamEmitter, DemuxedConsumableStream, StreamDemux } from '@topgunbuild/async-stream-emitter';
import { EventObject, SocketState } from '../types';
import { SimpleExchange } from '../simple-broker';
import { SocketServer } from './server';

export class Socket extends AsyncStreamEmitter<any>
{
    static CONNECTING: SocketState = 'connecting';
    static OPEN: SocketState       = 'open';
    static CLOSED: SocketState     = 'closed';

    static ignoreStatuses = socketProtocolIgnoreStatuses;
    static errorStatuses  = socketProtocolErrorStatuses;

    id: string;
    server: SocketServer;
    socket: any; // WebSocket;
    state: SocketState;
    request: {[key: string]: any};
    remoteAddress: string;
    remoteFamily: string;
    remotePort: number;
    forwardedForAddress?: string;
    channelSubscriptions: {
        [channelName: string]: boolean;
    };
    channelSubscriptionsCount: number;
    exchange: SimpleExchange;
    _handshakeTimeoutRef: any;

    CONNECTING: SocketState    = Socket.CONNECTING;
    OPEN: SocketState          = Socket.OPEN;
    CLOSED: SocketState        = Socket.CLOSED;

    private readonly _autoAckRPCs: {'#publish': number};
    private readonly _callbackMap: {[key: string]: any};
    private readonly _pingIntervalTicker: any;
    private _receiverDemux: StreamDemux<any>;
    private _procedureDemux: StreamDemux<any>;
    private _cid: number;
    private _batchSendList: any[];
    private _pingTimeoutTicker: any;
    private _batchTimeout: any;

    /**
     * Constructor
     */
    constructor(id: string, server: SocketServer, socket: any)
    {
        super();

        this._autoAckRPCs = {
            '#publish': 1
        };

        this.id        = id;
        this.server    = server;
        this.socket    = socket;
        this.state     = this.CONNECTING;

        this._receiverDemux  = new StreamDemux();
        this._procedureDemux = new StreamDemux();

        this.request = this.socket['upgradeReq'] || {};

        if (this.request.connection)
        {
            this.remoteAddress = this.request.connection.remoteAddress;
            this.remoteFamily  = this.request.connection.remoteFamily;
            this.remotePort    = this.request.connection.remotePort;
        }
        else
        {
            this.remoteAddress = this.request.remoteAddress;
            this.remoteFamily  = this.request.remoteFamily;
            this.remotePort    = this.request.remotePort;
        }
        if (this.request.forwardedForAddress)
        {
            this.forwardedForAddress = this.request.forwardedForAddress;
        }

        this._cid           = 1;
        this._callbackMap   = {};
        this._batchSendList = [];

        this.channelSubscriptions      = {};
        this.channelSubscriptionsCount = 0;

        this._on('error', async (err) =>
        {
            this.emitError(err);
        });

        this._on('close', async (code: number, reasonBuffer) =>
        {
            const reason = reasonBuffer && reasonBuffer.toString();
            this._onClose(code, reason);
        });

        if (!this.server.pingTimeoutDisabled)
        {
            this._pingIntervalTicker = setInterval(this._sendPing.bind(this), this.server.pingInterval);
        }
        this._resetPongTimeout();

        // Receive incoming raw messages
        this._on('message', async (msg) =>
        {
            const message = isNode() ? msg : msg.data;
            this._resetPongTimeout();

            this.emit('message', { message });

            let obj;
            try
            {
                obj = this.decode(message);
            }
            catch (err)
            {
                if (err.name === 'Error')
                {
                    err.name = 'InvalidMessageError';
                }
                this.emitError(err);
                return;
            }

            // If pong
            if (obj === '#2')
            {
            }
            else
            {
                if (Array.isArray(obj))
                {
                    const len = obj.length;
                    for (let i = 0; i < len; i++)
                    {
                        this._handleRemoteEventObject(obj[i], message);
                    }
                }
                else
                {
                    this._handleRemoteEventObject(obj, message);
                }
            }
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    receiver(receiverName: string): DemuxedConsumableStream<any>
    {
        return this._receiverDemux.stream(receiverName);
    }

    closeReceiver(receiverName: string): void
    {
        this._receiverDemux.close(receiverName);
    }

    procedure(procedureName: string): DemuxedConsumableStream<any>
    {
        return this._procedureDemux.stream(procedureName);
    }

    closeProcedure(procedureName: string): void
    {
        this._procedureDemux.close(procedureName);
    }

    getState(): SocketState
    {
        return this.state;
    }

    getBytesReceived(): any
    {
        return this.socket?.bytesReceived;
    }

    emitError(error?: Error): void
    {
        this.emit('error', {
            error
        });
    }

    disconnect(code?: number, data?: any): void
    {
        code = code || 1000;

        if (typeof code !== 'number')
        {
            const err = new InvalidArgumentsError('If specified, the code argument must be a number');
            this.emitError(err);
        }

        if (this.state !== this.CLOSED)
        {
            this._onClose(code, data);
            this.socket.close(code, data);
        }
    }

    terminate(): void
    {
        this.socket.terminate();
    }

    send(data: any, options?: any): void
    {
        if (isNode())
        {
            this.socket.send(data, options, (error: Error) =>
            {
                if (error)
                {
                    this._onClose(1006, error.toString());
                }
            });
        }
        else
        {
            this.socket.send(data);
        }
    }

    decode(message: any): any
    {
        return this.server.codec.decode(message);
    }

    encode(object: any): any
    {
        return this.server.codec.encode(object);
    }

    sendObjectBatch(object: any): void
    {
        this._batchSendList.push(object);
        if (this._batchTimeout)
        {
            return;
        }

        this._batchTimeout = setTimeout(() =>
        {
            delete this._batchTimeout;
            if (this._batchSendList.length)
            {
                let str;
                try
                {
                    str = this.encode(this._batchSendList);
                }
                catch (err)
                {
                    this.emitError(err);
                }
                if (str != null)
                {
                    this.send(str);
                }
                this._batchSendList = [];
            }
        }, this.server.options.pubSubBatchDuration || 0);
    }

    sendObjectSingle(object: any): void
    {
        let str;
        try
        {
            str = this.encode(object);
        }
        catch (err)
        {
            this.emitError(err);
        }
        if (str != null)
        {
            this.send(str);
        }
    }

    sendObject(object: any, options?: {batch?: boolean}): void
    {
        if (options && options.batch)
        {
            this.sendObjectBatch(object);
        }
        else
        {
            this.sendObjectSingle(object);
        }
    }

    transmit(event: string, data: any, options?: any): Promise<void>
    {
        this.server.verifyOutboundEvent(this, event, data, options, (err, newData) =>
        {
            const eventObject: EventObject = {
                event: event
            };
            if (newData !== undefined)
            {
                eventObject.data = newData;
            }

            if (!err)
            {
                if (options && options.useCache && options.stringifiedData != null)
                {
                    // Optimized
                    this.send(options.stringifiedData);
                }
                else
                {
                    this.sendObject(eventObject);
                }
            }
        });
        return Promise.resolve();
    }

    invoke(event: string, data?: any, options?: any): Promise<any>
    {
        return new Promise((resolve, reject) =>
        {
            this.server.verifyOutboundEvent(this, event, data, options, (err, newData) =>
            {
                if (err)
                {
                    reject(err);
                    return;
                }
                const eventObject: EventObject = {
                    event: event,
                    cid  : this._nextCallId()
                };
                if (newData !== undefined)
                {
                    eventObject.data = newData;
                }

                const timeout = setTimeout(() =>
                {
                    const error = new TimeoutError(`Event response for "${event}" timed out`);
                    delete this._callbackMap[eventObject.cid];
                    reject(error);
                }, this.server.ackTimeout);

                this._callbackMap[eventObject.cid] = {
                    callback: (err, result) =>
                    {
                        if (err)
                        {
                            reject(err);
                            return;
                        }
                        resolve(result);
                    },
                    timeout : timeout
                };

                if (options && options.useCache && options.stringifiedData != null)
                {
                    // Optimized
                    this.send(options.stringifiedData);
                }
                else
                {
                    this.sendObject(eventObject);
                }
            });
        });
    }

    kickOut(channel?: string, message?: string): Promise<any>
    {
        if (channel == null)
        {
            Object.keys(this.channelSubscriptions).forEach((channelName) =>
            {
                delete this.channelSubscriptions[channelName];
                this.channelSubscriptionsCount--;
                this.transmit('#kickOut', { message: message, channel: channelName });
            });
        }
        else
        {
            delete this.channelSubscriptions[channel];
            this.channelSubscriptionsCount--;
            this.transmit('#kickOut', { message: message, channel: channel });
        }
        return this.server.brokerEngine.unsubscribeSocket(this, channel);
    }

    subscriptions(): string[]
    {
        return Object.keys(this.channelSubscriptions);
    };

    isSubscribed(channel: string): boolean
    {
        return !!this.channelSubscriptions[channel];
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _onClose(code?: number, reason?: any): void
    {
        clearInterval(this._pingIntervalTicker);
        clearTimeout(this._pingTimeoutTicker);

        if (this.state !== this.CLOSED)
        {
            const prevState = this.state;
            this.state    = this.CLOSED;

            if (prevState === this.CONNECTING)
            {
                this.emit('connectAbort', { code, reason });
            }
            else
            {
                this.emit('disconnect', { code, reason });
            }
            this.emit('close', { code, reason });

            if (!Socket.ignoreStatuses[code])
            {
                let closeMessage;
                if (reason)
                {
                    let reasonString;
                    if (typeof reason === 'object')
                    {
                        try
                        {
                            reasonString = JSON.stringify(reason);
                        }
                        catch (error)
                        {
                            reasonString = reason.toString();
                        }
                    }
                    else
                    {
                        reasonString = reason;
                    }
                    closeMessage = `Socket connection closed with status code ${code} and reason: ${reasonString}`;
                }
                else
                {
                    closeMessage = `Socket connection closed with status code ${code}`;
                }
                const err = new SocketProtocolError(Socket.errorStatuses[code] || closeMessage, code);
                this.emitError(err);
            }
        }
    }

    private _sendPing(): void
    {
        if (this.state !== this.CLOSED)
        {
            this.sendObject('#1');
        }
    }

    private _handleRemoteEventObject(obj: any, message?: any): void
    {
        if (obj && obj.event != null)
        {
            const eventName = obj.event;

            const requestOptions: EventObject = {
                socket: this,
                event : eventName,
                data  : obj.data,
            };

            if (obj.cid == null)
            {
                this.server.verifyInboundRemoteEvent(requestOptions, (err, newEventData) =>
                {
                    if (!err)
                    {
                        this._receiverDemux.write(eventName, newEventData);
                    }
                });
            }
            else
            {
                requestOptions.cid = obj.cid;
                const response       = new Response(this, requestOptions.cid);
                this.server.verifyInboundRemoteEvent(requestOptions, (err, newEventData, ackData) =>
                {
                    if (err)
                    {
                        response.error(err);
                    }
                    else
                    {
                        if (this._autoAckRPCs[eventName])
                        {
                            if (ackData !== undefined)
                            {
                                response.end(ackData);
                            }
                            else
                            {
                                response.end();
                            }
                        }
                        else
                        {
                            this._procedureDemux.write(eventName, {
                                data : newEventData,
                                end  : (data) =>
                                {
                                    response.end(data);
                                },
                                error: (err) =>
                                {
                                    response.error(err);
                                }
                            });
                        }
                    }
                });
            }
        }
        else if (obj && obj.rid != null)
        {
            // If incoming message is a response to a previously sent message
            const ret = this._callbackMap[obj.rid];
            if (ret)
            {
                clearTimeout(ret.timeout);
                delete this._callbackMap[obj.rid];
                const rehydratedError = hydrateError(obj.error);
                ret.callback(rehydratedError, obj.data);
            }
        }
        else
        {
            // The last remaining case is to treat the message as raw
            this.emit('raw', { message });
        }
    }

    private _resetPongTimeout(): void
    {
        if (this.server.pingTimeoutDisabled)
        {
            return;
        }
        clearTimeout(this._pingTimeoutTicker);
        this._pingTimeoutTicker = setTimeout(() =>
        {
            this._onClose(4001);
            this.socket.close(4001);
        }, this.server.pingTimeout);
    }

    private _nextCallId(): number
    {
        return this._cid++;
    }

    /**
     * Listen websocket
     */
    private _on(
        event: 'message',
        cb: (messageBuffer: any, isBinary: boolean) => Promise<any>
    ): void;
    private _on(
        event: 'close',
        cb: (code: number, reasonBuffer: any) => Promise<any>
    ): void;
    private _on(event: 'error', cb: (error: any) => Promise<any>): void;
    private _on(
        event: 'message'|'close'|'error',
        cb: (arg1: any, arg2?: any) => Promise<any>
    ): void
    {
        if (isNode())
        {
            this.socket['on'](event, cb);
        }
        else
        {
            this.socket['addEventListener'](event, cb);
        }
    }
}
