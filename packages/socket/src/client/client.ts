import { AsyncStreamEmitter, DemuxedConsumableStream, StreamDemux } from '@topgunbuild/async-stream-emitter';
import { IClientSocket, SocketClientOptions, SubscribeOptions, TransmitOptions } from './types';
import { CodecEngine, EventObject, SocketState } from '../types';
import { SocketProtocolErrorStatuses, SocketProtocolIgnoreStatuses } from '../errors/types';
import {
    BadConnectionError,
    InvalidArgumentsError, SocketProtocolError,
    socketProtocolErrorStatuses,
    socketProtocolIgnoreStatuses,
    TimeoutError,
} from '../errors/errors';
import { Transport } from './transport';
import { Item, LinkedList } from './linked-list';
import { Response } from '../response';
import { defaultCodecEngine } from '../default-codec-engine';
import { Channel, ChannelOptions, ChannelState } from '../channel';

const isBrowser = typeof window !== 'undefined';

export class ClientSocket extends AsyncStreamEmitter<any> implements IClientSocket
{
    static readonly CONNECTING: SocketState = 'connecting';
    static readonly OPEN: SocketState       = 'open';
    static readonly CLOSED: SocketState     = 'closed';

    static readonly SUBSCRIBED   = 'subscribed';
    static readonly PENDING      = 'pending';
    static readonly UNSUBSCRIBED = 'unsubscribed';

    static readonly ignoreStatuses: SocketProtocolIgnoreStatuses = socketProtocolIgnoreStatuses;
    static readonly errorStatuses: SocketProtocolErrorStatuses   = socketProtocolErrorStatuses;

    options: SocketClientOptions;

    id: string|null;
    clientId?: string|undefined;

    version: string|null;

    state: SocketState;

    pendingReconnect: boolean;
    pendingReconnectTimeout: number;

    preparingPendingSubscriptions: boolean;

    ackTimeout: number;
    connectTimeout: number;

    pingTimeout: number;
    pingTimeoutDisabled: boolean;

    channelPrefix: string|null;
    disconnectOnUnload: boolean;

    connectAttempts: number;

    codec: CodecEngine;
    transport: Transport|undefined;

    private _reconnectTimeoutRef: any;
    private readonly _transmitBuffer: LinkedList<any>;
    private readonly _channelMap: { [key: string]: any };
    private _cid: number;
    private readonly _procedureDemux: StreamDemux<unknown>;
    private readonly _receiverDemux: StreamDemux<unknown>;
    private readonly _channelDataDemux: StreamDemux<unknown>;
    private readonly _channelEventDemux: StreamDemux<unknown>;
    private readonly _privateDataHandlerMap: {
        '#publish': (data: any) => void,
        '#kickOut': (data?: any) => void;
    };

    /**
     * Constructor
     */
    constructor(socketOptions?: SocketClientOptions)
    {
        super();
        socketOptions = socketOptions || {};

        const defaultOptions: SocketClientOptions = {
            path                  : '/topgunsocket/',
            secure                : false,
            autoConnect           : true,
            autoReconnect         : true,
            autoSubscribeOnConnect: true,
            connectTimeout        : 20000,
            ackTimeout            : 10000,
            timestampRequests     : false,
            timestampParam        : 't',
            binaryType            : 'arraybuffer',
            cloneData             : false,
        };

        const opts = Object.assign(defaultOptions, socketOptions);

        this.id                            = null;
        this.version                       = opts.version || null;
        this.state                         = ClientSocket.CLOSED;
        this.pendingReconnect              = false;
        this.pendingReconnectTimeout       = null;
        this.preparingPendingSubscriptions = false;
        this.clientId                      = opts.clientId;

        this.connectTimeout     = opts.connectTimeout;
        this.ackTimeout         = opts.ackTimeout;
        this.channelPrefix      = opts.channelPrefix || null;
        this.disconnectOnUnload = opts.disconnectOnUnload == null ? true : opts.disconnectOnUnload;

        // pingTimeout will be connectTimeout at the start, but it will
        // be updated with values provided by the 'connect' event
        opts.pingTimeout         = opts.connectTimeout;
        this.pingTimeout         = opts.pingTimeout;
        this.pingTimeoutDisabled = !!opts.pingTimeoutDisabled;

        const maxTimeout = Math.pow(2, 31) - 1;

        const verifyDuration = (propertyName) =>
        {
            if (this[propertyName] > maxTimeout)
            {
                throw new InvalidArgumentsError(
                    `The ${propertyName} value provided exceeded the maximum amount allowed`,
                );
            }
        };

        verifyDuration('connectTimeout');
        verifyDuration('ackTimeout');
        verifyDuration('pingTimeout');

        this.connectAttempts = 0;

        this._transmitBuffer = new LinkedList();
        this._channelMap     = {};

        this._channelEventDemux = new StreamDemux();
        this._channelDataDemux  = new StreamDemux();

        this._receiverDemux  = new StreamDemux();
        this._procedureDemux = new StreamDemux();

        this.options = opts;
        this._cid    = 1;

        this.options.callIdGenerator = () =>
        {
            return this._cid++;
        };

        if (this.options.autoReconnect)
        {
            if (this.options.autoReconnectOptions == null)
            {
                this.options.autoReconnectOptions = {};
            }

            // Add properties to the this.options.autoReconnectOptions object.
            // We assign the reference to a reconnectOptions variable to avoid repetition.
            const reconnectOptions = this.options.autoReconnectOptions;
            if (reconnectOptions.initialDelay == null)
            {
                reconnectOptions.initialDelay = 10000;
            }
            if (reconnectOptions.randomness == null)
            {
                reconnectOptions.randomness = 10000;
            }
            if (reconnectOptions.multiplier == null)
            {
                reconnectOptions.multiplier = 1.5;
            }
            if (reconnectOptions.maxDelay == null)
            {
                reconnectOptions.maxDelay = 60000;
            }
        }

        if (this.options.subscriptionRetryOptions == null)
        {
            this.options.subscriptionRetryOptions = {};
        }

        if (this.options.codecEngine)
        {
            this.codec = this.options.codecEngine;
        }
        else
        {
            this.codec = defaultCodecEngine;
        }

        if (this.options.protocol)
        {
            const protocolOptionError = new InvalidArgumentsError(
                'The "protocol" option does not affect asyngular-client - ' +
                'If you want to utilize SSL/TLS, use "secure" option instead',
            );
            this._onError(protocolOptionError);
        }

        this.options.path = this.options.path.replace(/\/$/, '') + '/';

        this.options.query = opts.query || {};
        if (typeof this.options.query === 'string')
        {
            const searchParams                        = new URLSearchParams(this.options.query);
            const queryObject: { [key: string]: any } = {};

            searchParams.forEach((value, key) =>
            {
                const currentValue = queryObject[key];
                if (currentValue == null)
                {
                    queryObject[key] = value;
                }
                else
                {
                    if (!Array.isArray(currentValue))
                    {
                        queryObject[key] = [currentValue];
                    }
                    queryObject[key].push(value);
                }
            });

            this.options.query = queryObject;
        }

        if (isBrowser && this.disconnectOnUnload && global.addEventListener && global.removeEventListener)
        {
            this._handleBrowserUnload();
        }

        if (this.options.autoConnect)
        {
            this.connect();
        }

        this._privateDataHandlerMap = {
            '#publish': (data) =>
            {
                const undecoratedChannelName = this._undecorateChannelName(data.channel);
                const isSubscribed           = this.isSubscribed(undecoratedChannelName, true);

                if (isSubscribed)
                {
                    this._channelDataDemux.write(undecoratedChannelName, data.data);
                }
            },
            '#kickOut': (data) =>
            {
                const undecoratedChannelName = this._undecorateChannelName(data.channel);
                const channel                = this._channelMap[undecoratedChannelName];
                if (channel)
                {
                    this.emit('kickOut', {
                        channel: undecoratedChannelName,
                        message: data.message,
                    });
                    this._channelEventDemux.write(`${undecoratedChannelName}/kickOut`, { message: data.message });
                    this._triggerChannelUnsubscribe(channel);
                }
            },
        };
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    getState(): SocketState
    {
        return this.state;
    }

    getBytesReceived(): number
    {
        return this.transport.getBytesReceived();
    }

    connect(): void
    {
        if (this.state === ClientSocket.CLOSED)
        {
            this.pendingReconnect        = false;
            this.pendingReconnectTimeout = null;
            clearTimeout(this._reconnectTimeoutRef);

            this.state = ClientSocket.CONNECTING;
            this.emit('connecting', {});

            if (this.transport)
            {
                this.transport.closeAllListeners();
            }

            const transport = new Transport(this.codec, this.options);
            this.transport  = transport;

            (async () =>
            {
                const asyncIterator = transport.listener('open').createConsumer();
                while (true)
                {
                    const packet = await asyncIterator.next();
                    if (packet.done) break;
                    this.state = ClientSocket.OPEN;
                    this._onOpen(packet.value);
                }
            })();

            (async () =>
            {
                const asyncIterator = transport.listener('error').createConsumer();
                while (true)
                {
                    const packet = await asyncIterator.next();
                    if (packet.done) break;
                    this._onError(packet.value.error);
                }
            })();

            (async () =>
            {
                const asyncIterator = transport.listener('close').createConsumer();
                while (true)
                {
                    const packet = await asyncIterator.next();
                    if (packet.done) break;
                    this.state = ClientSocket.CLOSED;
                    this._onClose(packet.value.code, packet.value.data);
                }
            })();

            (async () =>
            {
                const asyncIterator = transport.listener('openAbort').createConsumer();
                while (true)
                {
                    const packet = await asyncIterator.next();
                    if (packet.done) break;
                    this.state = ClientSocket.CLOSED;
                    this._onClose(packet.value.code, packet.value.data, true);
                }
            })();

            (async () =>
            {
                const asyncIterator = transport.listener('event').createConsumer();
                while (true)
                {
                    const packet = await asyncIterator.next();
                    if (packet.done) break;
                    this.emit(packet.value.event, packet.value.data);
                }
            })();

            (async () =>
            {
                const asyncIterator = transport.listener('inboundTransmit').createConsumer();
                while (true)
                {
                    const packet = await asyncIterator.next();
                    if (packet.done) break;
                    this._onInboundTransmit(packet.value.event, packet.value.data);
                }
            })();

            (async () =>
            {
                const asyncIterator = transport.listener('inboundInvoke').createConsumer();
                while (true)
                {
                    const packet = await asyncIterator.next();
                    if (packet.done) break;
                    this._onInboundInvoke(packet.value.event, packet.value.data, packet.value.response);
                }
            })();
        }
    }

    reconnect(code?: number, reason?: string): void
    {
        this.disconnect(code, reason);
        this.connect();
    }

    disconnect(code?: number, reason?: string): void
    {
        code = code || 1000;

        if (typeof code !== 'number')
        {
            throw new InvalidArgumentsError('If specified, the code argument must be a number');
        }

        const isConnecting = this.state === ClientSocket.CONNECTING;
        if (isConnecting || this.state === ClientSocket.OPEN)
        {
            this.state = ClientSocket.CLOSED;
            this._onClose(code, reason, isConnecting);
            this.transport.close(code, reason);
        }
        else
        {
            this.pendingReconnect        = false;
            this.pendingReconnectTimeout = null;
            clearTimeout(this._reconnectTimeoutRef);
        }
    }

    decode(message: any): any
    {
        return this.transport.decode(message);
    }

    encode(object: any): any
    {
        return this.transport.encode(object);
    }

    send(data: any): void
    {
        this.transport.send(data);
    }

    transmit(
        event: string,
        data?: any,
    ): Promise<void>
    {
        return this._processOutboundEvent(event, data);
    }

    invoke(
        event: string,
        data: any,
    ): Promise<any>
    {
        return this._processOutboundEvent(event, data, true);
    }

    publish(channelName: string, data?: any): Promise<any>
    {
        const pubData = {
            channel: this._decorateChannelName(channelName),
            data   : data,
        };
        return this.invoke('#publish', pubData);
    }

    subscribe(channelName: string, options?: SubscribeOptions): Channel<any>
    {
        options     = options || {};
        let channel = this._channelMap[channelName];

        const sanitizedOptions: SubscribeOptions = {
            batch      : !!options.batch,
        };

        if (options.priority != null)
        {
            sanitizedOptions.priority = options.priority;
        }
        if (options.data !== undefined)
        {
            sanitizedOptions.data = options.data;
        }

        if (!channel)
        {
            channel                       = {
                name   : channelName,
                state  : Channel.PENDING,
                options: sanitizedOptions,
            };
            this._channelMap[channelName] = channel;
            this._trySubscribe(channel);
        }
        else if (options)
        {
            channel.options = sanitizedOptions;
        }

        const channelDataStream = this._channelDataDemux.stream(channelName);
        return new Channel(
            channelName,
            this,
            this._channelEventDemux,
            channelDataStream,
        );
    }

    unsubscribe(channelName: string): void
    {
        const channel = this._channelMap[channelName];

        if (channel)
        {
            this._triggerChannelUnsubscribe(channel);
            this._tryUnsubscribe(channel);
        }
    }

    channel(channelName: string): Channel<any>
    {
        const channelDataStream = this._channelDataDemux.stream(channelName);
        return new Channel(
            channelName,
            this,
            this._channelEventDemux,
            channelDataStream,
        );
    }

    getChannelState(channelName: string): ChannelState
    {
        const channel = this._channelMap[channelName];
        if (channel)
        {
            return channel.state;
        }
        return Channel.UNSUBSCRIBED;
    }

    getChannelOptions(channelName: string): ChannelOptions
    {
        const channel = this._channelMap[channelName];
        if (channel)
        {
            return { ...channel.options };
        }
        return {};
    }

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

    closeChannel(channelName: string): void
    {
        this._channelDataDemux.close(channelName);
    }

    subscriptions(includePending?: boolean): string[]
    {
        const subs = [];
        Object.keys(this._channelMap).forEach((channelName) =>
        {
            if (includePending || this._channelMap[channelName].state === Channel.SUBSCRIBED)
            {
                subs.push(channelName);
            }
        });
        return subs;
    }

    isSubscribed(channelName: string, includePending?: boolean): boolean
    {
        const channel = this._channelMap[channelName];
        if (includePending)
        {
            return !!channel;
        }
        return !!channel && channel.state === Channel.SUBSCRIBED;
    }

    processPendingSubscriptions(): void
    {
        this.preparingPendingSubscriptions = false;
        const pendingChannels              = [];

        Object.keys(this._channelMap).forEach((channelName) =>
        {
            const channel = this._channelMap[channelName];
            if (channel.state === Channel.PENDING)
            {
                pendingChannels.push(channel);
            }
        });

        pendingChannels.sort((a, b) =>
        {
            const ap = a.options.priority || 0;
            const bp = b.options.priority || 0;
            if (ap > bp)
            {
                return -1;
            }
            if (ap < bp)
            {
                return 1;
            }
            return 0;
        });

        pendingChannels.forEach((channel) =>
        {
            this._trySubscribe(channel);
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _tryUnsubscribe(channel: Channel<any>): void
    {
        if (this.state === ClientSocket.OPEN)
        {
            const options: TransmitOptions = {
                noTimeout: true,
            };
            if (channel.options.batch)
            {
                options.batch = true;
            }
            // If there is a pending subscribe action, cancel the callback
            this._cancelPendingSubscribeCallback(channel);

            // This operation cannot fail because the TCP protocol guarantees delivery
            // so long as the connection remains open. If the connection closes,
            // the server will automatically unsubscribe the client and thus complete
            // the operation on the server side.
            const decoratedChannelName = this._decorateChannelName(channel.name);
            this.transport.transmit('#unsubscribe', decoratedChannelName, options);
        }
    }

    private _triggerChannelUnsubscribe(channel: Channel<any>, setAsPending?: boolean): void
    {
        const channelName = channel.name;

        this._cancelPendingSubscribeCallback(channel);

        if (channel.state === Channel.SUBSCRIBED)
        {
            const stateChangeData = {
                oldChannelState: channel.state,
                newChannelState: setAsPending ? Channel.PENDING : Channel.UNSUBSCRIBED,
            };
            this._channelEventDemux.write(`${channelName}/subscribeStateChange`, stateChangeData);
            this._channelEventDemux.write(`${channelName}/unsubscribe`, {});
            this.emit('subscribeStateChange', {
                channel: channelName,
                ...stateChangeData,
            });
            this.emit('unsubscribe', { channel: channelName });
        }

        if (setAsPending)
        {
            channel.state = Channel.PENDING;
        }
        else
        {
            delete this._channelMap[channelName];
        }
    }

    private _trySubscribe(channel: Channel<any>): void
    {
        // We can only ever have one pending subscribe action at any given time on a channel
        if (
            this.state === ClientSocket.OPEN &&
            !this.preparingPendingSubscriptions &&
            channel._pendingSubscriptionCid == null
        )
        {

            const options: { [key: string]: any } = {
                noTimeout: true,
            };

            const subscriptionOptions: SubscribeOptions = {};
            if (channel.options.data)
            {
                subscriptionOptions.data = channel.options.data;
            }
            if (channel.options.batch)
            {
                options.batch             = true;
                subscriptionOptions.batch = true;
            }

            channel._pendingSubscriptionCid = this.transport.invokeRaw(
                '#subscribe',
                {
                    channel: this._decorateChannelName(channel.name),
                    ...subscriptionOptions,
                },
                options,
                (err) =>
                {
                    if (err)
                    {
                        if (err.name === 'BadConnectionError')
                        {
                            // In case of a failed connection, keep the subscription
                            // as pending; it will try again on reconnect.
                            return;
                        }
                        delete channel._pendingSubscriptionCid;
                        this._triggerChannelSubscribeFail(err, channel, subscriptionOptions);
                    }
                    else
                    {
                        delete channel._pendingSubscriptionCid;
                        this._triggerChannelSubscribe(channel, subscriptionOptions);
                    }
                },
            );
            this.emit('subscribeRequest', {
                channel: channel.name,
                subscriptionOptions,
            });
        }
    }

    private _undecorateChannelName(decoratedChannelName: string): string
    {
        if (this.channelPrefix && decoratedChannelName.indexOf(this.channelPrefix) === 0)
        {
            return decoratedChannelName.replace(this.channelPrefix, '');
        }
        return decoratedChannelName;
    }

    private _decorateChannelName(channelName: string): string
    {
        if (this.channelPrefix)
        {
            channelName = this.channelPrefix + channelName;
        }
        return channelName;
    }

    private _cancelPendingSubscribeCallback(channel: Channel<any>): void
    {
        if (channel._pendingSubscriptionCid != null)
        {
            this.transport.cancelPendingResponse(channel._pendingSubscriptionCid);
            delete channel._pendingSubscriptionCid;
        }
    }

    private _triggerChannelSubscribeFail(err: Error, channel: Channel<any>, subscriptionOptions: SubscribeOptions): void
    {
        const channelName = channel.name;
        const hasChannel  = !!this._channelMap[channelName];

        if (hasChannel)
        {
            delete this._channelMap[channelName];

            this._channelEventDemux.write(`${channelName}/subscribeFail`, {
                error: err,
                subscriptionOptions,
            });
            this.emit('subscribeFail', {
                error              : err,
                channel            : channelName,
                subscriptionOptions: subscriptionOptions,
            });
        }
    }

    private _triggerChannelSubscribe(channel: Channel<any>, subscriptionOptions: SubscribeOptions): void
    {
        const channelName = channel.name;

        if (channel.state !== Channel.SUBSCRIBED)
        {
            const oldChannelState = channel.state;
            channel.state         = Channel.SUBSCRIBED;

            const stateChangeData = {
                oldChannelState,
                newChannelState: channel.state,
                subscriptionOptions,
            };
            this._channelEventDemux.write(`${channelName}/subscribeStateChange`, stateChangeData);
            this._channelEventDemux.write(`${channelName}/subscribe`, {
                subscriptionOptions,
            });
            this.emit('subscribeStateChange', {
                channel: channelName,
                ...stateChangeData,
            });
            this.emit('subscribe', {
                channel: channelName,
                subscriptionOptions,
            });
        }
    }

    private _processOutboundEvent(event: string, data?: any, expectResponse?: boolean): Promise<any>
    {
        if (this.state === ClientSocket.CLOSED)
        {
            this.connect();
        }
        const eventObject: EventObject = {
            event: event,
        };

        let promise;

        if (expectResponse)
        {
            promise = new Promise((resolve, reject) =>
            {
                eventObject.callback = (err, data) =>
                {
                    if (err)
                    {
                        reject(err);
                        return;
                    }
                    resolve(data);
                };
            });
        }
        else
        {
            promise = Promise.resolve();
        }

        const eventNode: any = new Item();

        eventObject.data = data;
        eventNode.data   = eventObject;

        eventObject.timeout = setTimeout(() =>
        {
            this._handleEventAckTimeout(eventObject, eventNode);
        }, this.ackTimeout);

        this._transmitBuffer.append(eventNode);
        if (this.state === ClientSocket.OPEN)
        {
            this._flushTransmitBuffer();
        }
        return promise;
    }

    private _handleEventAckTimeout(eventObject: EventObject, eventNode: Item): void
    {
        if (eventNode)
        {
            eventNode.detach();
        }
        delete eventObject.timeout;

        const callback = eventObject.callback;
        if (callback)
        {
            delete eventObject.callback;
            const error = new TimeoutError(`Event response for "${eventObject.event}" timed out`);
            callback.call(eventObject, error, eventObject);
        }
        // Cleanup any pending response callback in the transport layer too.
        if (eventObject.cid)
        {
            this.transport.cancelPendingResponse(eventObject.cid);
        }
    }

    private _flushTransmitBuffer(): void
    {
        let currentNode = this._transmitBuffer.head;
        let nextNode;

        while (currentNode)
        {
            nextNode          = currentNode.next;
            const eventObject = currentNode.data;
            currentNode.detach();
            this.transport.transmitObject(eventObject);
            currentNode = nextNode;
        }
    }

    private _onInboundInvoke(event: string, data: any, response: Response): void
    {
        this._procedureDemux.write(event, {
            data,
            end  : (data) =>
            {
                response.end(data);
            },
            error: (err) =>
            {
                response.error(err);
            },
        });
    }

    private _onInboundTransmit(event: string, data: any): void
    {
        const handler = this._privateDataHandlerMap[event];
        if (handler)
        {
            handler.call(this, data);
        }
        else
        {
            this._receiverDemux.write(event, data);
        }
    }

    private _onClose(code?: number, reason?: string, openAbort?: boolean)
    {
        this.id = null;
        if (this.transport)
        {
            this.transport.closeAllListeners();
        }

        this.pendingReconnect        = false;
        this.pendingReconnectTimeout = null;
        clearTimeout(this._reconnectTimeoutRef);

        this._suspendSubscriptions();

        const donePromise = this.listener('close').once();
        this._abortAllPendingEventsDueToBadConnection(openAbort ? 'connectAbort' : 'disconnect', donePromise);

        // Try to reconnect
        // on server ping timeout (4000)
        // or on client pong timeout (4001)
        // or on close without status (1005)
        // or on handshake failure (4003)
        // or on handshake rejection (4008)
        // or on socket hung up (1006)
        if (this.options.autoReconnect)
        {
            if (code === 4000 || code === 4001 || code === 1005)
            {
                // If there is a ping or pong timeout or socket closes without
                // status, don't wait before trying to reconnect - These could happen
                // if the client wakes up after a period of inactivity and in this case we
                // want to re-establish the connection as soon as possible.
                this._tryReconnect(0);

                // Codes 4500 and above will be treated as permanent disconnects.
                // Socket will not try to auto-reconnect.
            }
            else if (code !== 1000 && code < 4500)
            {
                this._tryReconnect();
            }
        }

        if (openAbort)
        {
            this.emit('connectAbort', { code, reason });
        }
        else
        {
            this.emit('disconnect', { code, reason });
        }
        this.emit('close', { code, reason });

        if (!ClientSocket.ignoreStatuses[code])
        {
            let closeMessage;
            if (reason)
            {
                closeMessage = 'Socket connection closed with status code ' + code + ' and reason: ' + reason;
            }
            else
            {
                closeMessage = 'Socket connection closed with status code ' + code;
            }
            const err = new SocketProtocolError(ClientSocket.errorStatuses[code] || closeMessage, code);
            this._onError(err);
        }
    }

    private _abortAllPendingEventsDueToBadConnection(failureType: string, donePromise: Promise<void>): void
    {
        let currentNode = this._transmitBuffer.head;
        let nextNode;

        while (currentNode)
        {
            nextNode          = currentNode.next;
            const eventObject = currentNode.data;
            clearTimeout(eventObject.timeout);
            delete eventObject.timeout;
            currentNode.detach();
            currentNode = nextNode;

            const callback = eventObject.callback;
            if (callback)
            {
                delete eventObject.callback;
                const errorMessage = `Event "${eventObject.event}" was aborted due to a bad connection`;
                const error        = new BadConnectionError(errorMessage, failureType);

                (async () =>
                {
                    await donePromise;
                    callback.call(eventObject, error, eventObject);
                })();
            }
            // Cleanup any pending response callback in the transport layer too.
            if (eventObject.cid)
            {
                this.transport.cancelPendingResponse(eventObject.cid);
            }
        }
    }

    private _suspendSubscriptions(): void
    {
        Object.keys(this._channelMap).forEach((channelName) =>
        {
            const channel = this._channelMap[channelName];
            this._triggerChannelUnsubscribe(channel, true);
        });
    }

    private _onError(error: any): void
    {
        this.emit('error', { error });
    }

    private _onOpen(status: any): void
    {
        this.preparingPendingSubscriptions = true;

        if (status)
        {
            this.id          = status.id;
            this.pingTimeout = status.pingTimeout;
        }

        this.connectAttempts = 0;

        if (this.options.autoSubscribeOnConnect)
        {
            this.processPendingSubscriptions();
        }

        // If the user invokes the callback while in autoSubscribeOnConnect mode, it
        // won't break anything.
        this.emit('connect', {
            ...status,
            processPendingSubscriptions: () =>
            {
                this.processPendingSubscriptions();
            },
        });

        if (this.state === ClientSocket.OPEN)
        {
            this._flushTransmitBuffer();
        }
    }

    private _tryReconnect(initialDelay?: number): void
    {
        const exponent         = this.connectAttempts++;
        const reconnectOptions = this.options.autoReconnectOptions;
        let timeout;

        if (initialDelay == null || exponent > 0)
        {
            const initialTimeout = Math.round(reconnectOptions.initialDelay + (reconnectOptions.randomness || 0) * Math.random());

            timeout = Math.round(initialTimeout * Math.pow(reconnectOptions.multiplier, exponent));
        }
        else
        {
            timeout = initialDelay;
        }

        if (timeout > reconnectOptions.maxDelay)
        {
            timeout = reconnectOptions.maxDelay;
        }

        clearTimeout(this._reconnectTimeoutRef);

        this.pendingReconnect        = true;
        this.pendingReconnectTimeout = timeout;
        this._reconnectTimeoutRef    = setTimeout(() =>
        {
            this.connect();
        }, timeout);
    }

    private async _handleBrowserUnload(): Promise<void>
    {
        const unloadHandler         = () =>
        {
            this.disconnect();
        };
        let isUnloadHandlerAttached = false;

        const attachUnloadHandler = () =>
        {
            if (!isUnloadHandlerAttached)
            {
                isUnloadHandlerAttached = true;
                global.addEventListener('beforeunload', unloadHandler, false);
            }
        };

        const detachUnloadHandler = () =>
        {
            if (isUnloadHandlerAttached)
            {
                isUnloadHandlerAttached = false;
                global.removeEventListener('beforeunload', unloadHandler, false);
            }
        };

        (async () =>
        {
            const asyncIterator = this.listener('connecting').createConsumer();
            while (true)
            {
                const packet = await asyncIterator.next();
                if (packet.done) break;
                attachUnloadHandler();
            }
        })();

        (async () =>
        {
            const asyncIterator = this.listener('close').createConsumer();
            while (true)
            {
                const packet = await asyncIterator.next();
                if (packet.done) break;
                detachUnloadHandler();
            }
        })();
    }
}
