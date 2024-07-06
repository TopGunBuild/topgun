import {
    Channel,
    ClientSocket,
    SocketClientOptions,
    create as createSocketClient, SubscribeOptions,
} from '@topgunbuild/socket';
import { Message } from '@topgunbuild/transport';
import { WireConnector } from './wire-connector';
import { ConnectorSendOptions, MessageCb } from '../types';

export class WebSocketConnector extends WireConnector
{
    client: ClientSocket;
    options: SocketClientOptions|undefined;

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
                await this.#onOutputProcessed(value);
            }
        })();
    }

    async disconnect(): Promise<void>
    {
        try
        {
            await super.disconnect();
            this.closeAllListeners();
            this.client.disconnect();
        }
        catch (e)
        {
            console.error(e);
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

    send(message: Message, options: ConnectorSendOptions): () => void
    {
        const msgId  = message.idString;
        const cbWrap = (msg: Message) =>
        {
            if (options?.cb)
            {
                options.cb(msg);
            }
            if (options?.once)
            {
                this.off(msgId);
            }
        };

        this._requestChannels[msgId] = this.#subscribeToChannel(
            `topgun/${msgId}`,
            cbWrap,
            {},
        );

        return super.send(message, {
            cb  : cbWrap,
            once: options.once,
        });
    }

    #subscribeToChannel(
        channelName: string,
        cb?: MessageCb,
        opts?: SubscribeOptions,
    ): Channel<any>
    {
        const channel = this.client.subscribe(channelName, opts);
        this.#onChannelMessage(channel, cb);
        return channel;
    }

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

    async #publishToChannel(channelName: string, message: Message): Promise<void>
    {
        try
        {
            if (message && this.client)
            {
                const messageId = message.idString;
                const channel   = this._requestChannels[messageId];

                if (channel && !channel.isSubscribed())
                {
                    await channel.listener('subscribe').once();
                }
                await this.client.publish(channelName, message);
            }
        }
        catch (e)
        {
            console.error(e);
        }
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

    async #onOutputProcessed(msg: Message): Promise<void>
    {
        if (msg && this.client)
        {
            const replyTo = msg.replyToIdString;

            if (replyTo)
            {
                await this.#publishToChannel(`topgun/@${replyTo}`, msg);
            }
            else
            {
                await this.#publishToChannel('topgun/message', msg);
            }
        }
    }
}

export function createConnector(
    opts: SocketClientOptions|undefined,
): WebSocketConnector
{
    return new WebSocketConnector(opts);
}
