import {
    Channel,
    ClientSocket,
    SocketClientOptions,
    create as createSocketClient, SubscribeOptions,
} from '@topgunbuild/socket';
import { Message } from '@topgunbuild/transport';
import { WireConnector } from './wire-connector';
import { MessageCb } from '../types';

export class WebSocketConnector extends WireConnector
{
    readonly client: ClientSocket;
    readonly options: SocketClientOptions|undefined;

    private readonly _requestChannels: {
        [msgId: string]: Channel<any>;
    };

    /**
     * Constructor
     */
    constructor(
        opts: SocketClientOptions|undefined,
        name = 'WebSocketGraphConnector',
    )
    {
        super(name);
        this._requestChannels = {};
        this.options          = opts;
        this.client           = createSocketClient(this.options || {});
        this.#onConnect();
        this.#onError();

        (async () =>
        {
            for await (const value of this.outputQueue.listener('completed'))
            {
                this.#onOutputProcessed(value);
            }
        })();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async disconnect(): Promise<void>
    {
        try
        {
            this.closeAllListeners();
            this.client.disconnect();
        }
        catch (e)
        {

        }
    }

    off(msgId: string): WebSocketConnector
    {
        super.off(msgId);
        const channel = this._requestChannels[msgId];

        if (channel)
        {
            channel.unsubscribe();
            delete this._requestChannels[msgId];
        }

        return this;
    }

    get(msg: Message, once: boolean, cb?: MessageCb): () => void
    {
        const msgId  = msg.idString;
        const cbWrap = (msg: Message) =>
        {
            if (cb)
            {
                cb(msg);
            }
            if (once)
            {
                this.off(msgId);
            }
        };

        this._requestChannels[msgId] = this.subscribeToChannel(
            `topgun/nodes/${msgId}`,
            cbWrap,
            {},
        );

        return super.sendMessage(msg, cbWrap);
    }

    put(msg: Message, cb?: MessageCb): () => void
    {
        const messageId = msg.idString;

        const cbWrap = (response: Message) =>
        {
            this.off(messageId);
            if (cb)
            {
                cb(response);
            }
        };

        this._requestChannels[messageId] = this.subscribeToChannel(
            `topgun/@${messageId}`,
            cbWrap,
        );

        return super.sendMessage(msg, cbWrap);
    }

    subscribeToChannel(
        channelName: string,
        cb?: MessageCb,
        opts?: SubscribeOptions,
    ): Channel<any>
    {
        const channel = this.client.subscribe(channelName, opts);
        this.#onChannelMessage(channel, cb);

        return channel;
    }

    rpc<T>(functionName: string, data?: any): Promise<T>
    {
        return this.client.invoke(functionName, data);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #onChannelMessage(
        channel: Channel<any>,
        cb?: MessageCb,
    ): Promise<void>
    {
        for await (const msg of channel)
        {
            this.ingest(msg);
            if (cb)
            {
                cb(msg);
            }
        }
    }

    #onOutputProcessed(message: Message): void
    {
        if (message && this.client)
        {
            const replyTo = message.replyToIdString;

            if (replyTo)
            {
                this.#publishToChannel(`topgun/@${replyTo}`, message);
            }
            else
            {
                this.#publishToChannel('topgun/message', message);
            }
        }
    }

    #publishToChannel(
        channelName: string,
        message: Message,
    ): WebSocketConnector
    {
        const messageId = message.idString;
        const channel   = this._requestChannels[messageId];

        if (channel)
        {
            channel
                .listener('subscribe')
                .once()
                .then(() =>
                {
                    this.client.publish(channelName, message);
                });
        }
        else
        {
            this.client.publish(channelName, message);
        }

        return this;
    }

    async #onConnect(): Promise<void>
    {
        for await (const _event of this.client.listener('connect'))
        {
            try
            {
                this.emit('connect', {});
            }
            catch (error)
            {
                console.error(error);
            }
        }
    }

    async #onError(): Promise<void>
    {
        for await (const _event of this.client.listener('error'))
        {
            console.error(
                'Socket Connection Error',
                _event.error.stack,
                _event.error.message,
            );
            this.emit('disconnect', {});
        }
    }
}

export function createConnector(
    opts: SocketClientOptions|undefined,
): WebSocketConnector
{
    return new WebSocketConnector(opts);
}
