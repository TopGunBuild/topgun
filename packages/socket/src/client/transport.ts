import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { CodecEngine, EventObject, EventObjectCallback, SocketState } from '../types';
import { InvokeOptions, SocketClientOptions, TransmitOptions } from './types';
import { createWebSocket } from '../utils/create-websocket';
import { BadConnectionError, hydrateError, TimeoutError } from '../errors/errors';
import { Response } from '../response';

export class Transport extends AsyncStreamEmitter<any>
{
    static CONNECTING: SocketState = 'connecting';
    static OPEN: SocketState       = 'open';
    static CLOSED: SocketState     = 'closed';

    CONNECTING = Transport.CONNECTING;
    OPEN       = Transport.OPEN;
    CLOSED     = Transport.CLOSED;

    state: SocketState;
    codec: CodecEngine;
    options: SocketClientOptions;
    connectTimeout: any;
    pingTimeout: any;
    pingTimeoutDisabled: boolean;
    callIdGenerator: any;
    socket: any;
    readonly _callbackMap: {
        [cid: number]: EventObject;
    };

    private _pingTimeoutTicker: any;
    private _batchSendList: any[];
    private readonly _connectTimeoutRef: any;
    private _batchTimeout: any;

    /**
     * Constructor
     */
    constructor(codecEngine: CodecEngine, options: SocketClientOptions)
    {
        super();
        this.state               = this.CLOSED;
        this.codec               = codecEngine;
        this.options             = options;
        this.connectTimeout      = options.connectTimeout;
        this.pingTimeout         = options.pingTimeout;
        this.pingTimeoutDisabled = !!options.pingTimeoutDisabled;
        this.callIdGenerator     = options.callIdGenerator;

        this._pingTimeoutTicker = null;
        this._callbackMap       = {};
        this._batchSendList     = [];

        // Open the connection.

        this.state = this.CONNECTING;
        const uri  = this.uri();

        const wsSocket         = createWebSocket(uri, this.options);
        wsSocket['binaryType'] = this.options.binaryType;

        this.socket = wsSocket;

        wsSocket.onopen = () =>
        {
            this._onOpen();
        };

        wsSocket.onclose = async (event) =>
        {
            let code;
            if (event.code == null)
            {
                // This is to handle an edge case in React Native whereby
                // event.code is undefined when the mobile device is locked.
                // TODO: This is not ideal since this condition could also apply to
                // an abnormal close (no close control frame) which would be a 1006.
                code = 1005;
            }
            else
            {
                code = event.code;
            }
            this._onClose(code, event.reason);
        };

        wsSocket.onmessage = (message) =>
        {
            this._onMessage(message.data);
        };

        wsSocket.onerror = () =>
        {
            // The onclose event will be called automatically after the onerror event
            // if the socket is connected - Otherwise, if it's in the middle of
            // connecting, we want to close it manually with a 1006 - This is necessary
            // to prevent inconsistent behavior when running the client in Node.js
            // vs in a browser.
            if (this.state === this.CONNECTING)
            {
                this._onClose(1006);
            }
        };

        this._connectTimeoutRef = setTimeout(() =>
        {
            this._onClose(4007);
            this.socket.close(4007);
        }, this.connectTimeout);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    uri(): string
    {
        let query: string|{ [key: string]: string|number|boolean } =
                this.options.query || {};
        const schema                                               = this.options.secure ? 'wss' : 'ws';

        if (this.options.timestampRequests)
        {
            query[this.options.timestampParam] = (new Date()).getTime();
        }

        const searchParams = new URLSearchParams();
        for (const [key, value] of Object.entries(query))
        {
            if (Array.isArray(value))
            {
                for (const item of value)
                {
                    searchParams.append(key, item);
                }
            }
            else
            {
                searchParams.set(key, `${value}`);
            }
        }

        query = searchParams.toString();

        if (query.length)
        {
            query = '?' + query;
        }

        let host;
        if (this.options.host)
        {
            host = this.options.host;
        }
        else
        {
            let port = '';

            if (this.options.port && ((schema === 'wss' && this.options.port !== 443)
                || (schema === 'ws' && this.options.port !== 80)))
            {
                port = ':' + this.options.port;
            }
            host = this.options.hostname + port;
        }

        return schema + '://' + host + this.options.path + query;
    }

    getBytesReceived(): any
    {
        return this.socket.bytesReceived;
    }

    close(code?: number, data?: any): void
    {
        if (this.state === this.OPEN || this.state === this.CONNECTING)
        {
            code = code || 1000;
            this._onClose(code, data);
            this.socket.close(code, data);
        }
    }

    transmitObject(eventObject: EventObject, options?: any): number
    {
        const simpleEventObject: EventObject = {
            event: eventObject.event,
            data : eventObject.data,
        };

        if (eventObject.callback)
        {
            simpleEventObject.cid              = eventObject.cid = this.callIdGenerator();
            this._callbackMap[eventObject.cid] = eventObject;
        }

        this.sendObject(simpleEventObject, options);

        return eventObject.cid || null;
    }

    transmit(
        event: string,
        data: any,
        options: TransmitOptions,
    ): Promise<void>
    {
        const eventObject = {
            event: event,
            data : data,
        };

        if (this.state === this.OPEN || options.force)
        {
            this.transmitObject(eventObject, options);
        }
        return Promise.resolve();
    }

    invokeRaw(
        event: string,
        data: any,
        options: InvokeOptions,
        callback?: EventObjectCallback,
    ): number|null
    {
        const eventObject: EventObject = {
            event   : event,
            data    : data,
            callback: callback,
        };

        if (!options.noTimeout)
        {
            eventObject.timeout = setTimeout(() =>
            {
                this._handleEventAckTimeout(eventObject);
            }, this.options.ackTimeout);
        }
        let cid = null;
        if (this.state === this.OPEN || options.force)
        {
            cid = this.transmitObject(eventObject, options);
        }
        return cid;
    }

    invoke<T>(
        event: string,
        data: T,
        options: InvokeOptions,
    ): Promise<EventObject>
    {
        return new Promise((resolve, reject) =>
        {
            this.invokeRaw(event, data, options, (err, data) =>
            {
                if (err)
                {
                    reject(err);
                    return;
                }
                resolve(data);
            });
        });
    }

    cancelPendingResponse(cid: number): void
    {
        delete this._callbackMap[cid];
    }

    decode(message: any): any
    {
        return this.codec.decode(message);
    }

    encode(object: any): any
    {
        return this.codec.encode(object);
    }

    send(data: any): void
    {
        if (this.socket.readyState !== this.socket.OPEN)
        {
            this._onClose(1005);
        }
        else
        {
            this.socket.send(data);
        }
    }

    serializeObject(object: any): string
    {
        let str, formatError;
        try
        {
            str = this.encode(object);
        }
        catch (err)
        {
            formatError = err;
            this._onError(formatError);
        }
        if (!formatError)
        {
            return str;
        }
        return null;
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
                const str = this.serializeObject(this._batchSendList);
                if (str != null)
                {
                    this.send(str);
                }
                this._batchSendList = [];
            }
        }, this.options.pubSubBatchDuration || 0);
    }

    sendObjectSingle(object: any): void
    {
        const str = this.serializeObject(object);
        if (str != null)
        {
            this.send(str);
        }
    }

    sendObject(object: any, options?: { batch?: boolean }): void
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

    _resetPingTimeout(): void
    {
        if (this.pingTimeoutDisabled)
        {
            return;
        }

        clearTimeout(this._pingTimeoutTicker);
        this._pingTimeoutTicker = setTimeout(() =>
        {
            this._onClose(4000);
            this.socket.close(4000);
        }, this.pingTimeout);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async _onOpen(): Promise<void>
    {
        clearTimeout(this._connectTimeoutRef);
        this._resetPingTimeout();

        let status;

        try
        {
            status = await this._handshake();
        }
        catch (err)
        {
            if (err.statusCode == null)
            {
                err.statusCode = 4003;
            }
            this._onError(err);
            this._onClose(err.statusCode, err.toString());
            this.socket.close(err.statusCode);
            return;
        }

        this.state = this.OPEN;
        if (status)
        {
            this.pingTimeout = status.pingTimeout;
        }
        this.emit('open', status);
        this._resetPingTimeout();
    }

    private async _handshake(): Promise<EventObject>
    {
        const options = {
            force: true,
        };
        return await this.invoke('#handshake', {}, options);
    }

    private _abortAllPendingEventsDueToBadConnection(failureType: string, donePromise: Promise<any>): void
    {
        Object.keys(this._callbackMap || {}).forEach((i) =>
        {
            const eventObject = this._callbackMap[i];
            delete this._callbackMap[i];

            clearTimeout(eventObject.timeout);
            delete eventObject.timeout;

            const errorMessage       = `Event "${eventObject.event}" was aborted due to a bad connection`;
            const badConnectionError = new BadConnectionError(errorMessage, failureType);

            const callback = eventObject.callback;
            delete eventObject.callback;

            (async () =>
            {
                await donePromise;
                callback.call(eventObject, badConnectionError, eventObject);
            })();
        });
    }

    private _onClose(code: number, data?: any): void
    {
        delete this.socket.onopen;
        delete this.socket.onclose;
        delete this.socket.onmessage;
        delete this.socket.onerror;

        clearTimeout(this._connectTimeoutRef);
        clearTimeout(this._pingTimeoutTicker);
        clearTimeout(this._batchTimeout);

        if (this.state === this.OPEN)
        {
            this.state        = this.CLOSED;
            const donePromise = this.listener('close').once();
            this._abortAllPendingEventsDueToBadConnection('disconnect', donePromise);
            this.emit('close', { code, data });
        }
        else if (this.state === this.CONNECTING)
        {
            this.state        = this.CLOSED;
            const donePromise = this.listener('openAbort').once();
            this._abortAllPendingEventsDueToBadConnection('connectAbort', donePromise);
            this.emit('openAbort', { code, data });
        }
    }

    private _handleTransmittedEventObject(obj: EventObject, message: any): void
    {
        if (obj && obj.event != null)
        {
            if (obj.cid == null)
            {
                this.emit('inboundTransmit', { ...obj });
            }
            else
            {
                const response = new Response(this, obj.cid);
                this.emit('inboundInvoke', { ...obj, response });
            }
        }
        else if (obj && obj.rid != null)
        {
            const eventObject = this._callbackMap[obj.rid];
            if (eventObject)
            {
                clearTimeout(eventObject.timeout);
                delete eventObject.timeout;
                delete this._callbackMap[obj.rid];

                if (eventObject.callback)
                {
                    const rehydratedError = hydrateError(obj.error);
                    eventObject.callback(rehydratedError, obj.data);
                }
            }
        }
        else
        {
            this.emit('event', { event: 'raw', data: { message } });
        }
    }

    private _onMessage(message: any): void
    {
        this.emit('event', { event: 'message', data: { message } });

        const obj = this.decode(message);

        // If ping
        if (obj === '#1')
        {
            this._resetPingTimeout();
            if (this.socket.readyState === this.socket.OPEN)
            {
                this.sendObject('#2');
            }
        }
        else
        {
            if (Array.isArray(obj))
            {
                const len = obj.length;
                for (let i = 0; i < len; i++)
                {
                    this._handleTransmittedEventObject(obj[i], message);
                }
            }
            else
            {
                this._handleTransmittedEventObject(obj, message);
            }
        }
    }

    private _onError(error: Error): void
    {
        this.emit('error', { error });
    }

    private _handleEventAckTimeout(eventObject: EventObject): void
    {
        if (eventObject.cid)
        {
            delete this._callbackMap[eventObject.cid];
        }
        delete eventObject.timeout;

        const callback = eventObject.callback;
        if (callback)
        {
            delete eventObject.callback;
            const error = new TimeoutError(`Event response for "${eventObject.event}" timed out`);
            callback.call(eventObject, error, eventObject);
        }
    }
}
