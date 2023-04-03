import { TGChannel } from 'topgun-socket';
import {
    ClientOptions,
    TGClientSocket,
    create as createSocketClient,
    SubscribeOptions
} from 'topgun-socket/client';
import { sign } from '../sea';
import { Get, Put, Message, MessageCb } from '../types';
import { GraphWireConnector } from '../client/transports/graph-wire-connector';
import { generateMessageId } from '../client/graph/graph-utils';

export class SocketConnector extends GraphWireConnector
{
    public readonly socket: TGClientSocket;
    public readonly opts: ClientOptions|undefined;
    public readonly msgChannel?: TGChannel<any>;
    public readonly getsChannel?: TGChannel<any>;
    public readonly putsChannel?: TGChannel<any>;

    private readonly _requestChannels: {
        [msgId: string]: TGChannel<any>;
    };

    /**
     * Constructor
     */
    constructor(
        opts: ClientOptions|undefined,
        name = 'SocketConnector'
    )
    {
        super(name);
        this._requestChannels = {};
        this.opts             = opts;
        this.socket           = createSocketClient(this.opts);
        this.onConnect();
        this.onError();
        this.outputQueue.completed.on(this.onOutputProcessed.bind(this));
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    off(msgId: string): SocketConnector
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

    get({
            soul,
            msgId,
            cb,
            opts
        }: Get
    ): () => void
    {
        const cbWrap = (msg: any) =>
        {
            this.ingest([msg]);
            if (cb)
            {
                cb(msg);
            }
        };

        msgId                        = msgId || generateMessageId();
        this._requestChannels[msgId] = this.subscribeToChannel(`topgun/nodes/${soul}`, cbWrap);

        return super.get({ soul, msgId, cb, opts });
    }

    put({
            graph,
            msgId = '',
            replyTo = '',
            cb
        }: Put
    ): () => void
    {
        if (!graph)
        {
            return () =>
            {
            };
        }

        msgId = msgId || generateMessageId();

        if (cb)
        {
            const cbWrap = (response: any) =>
            {
                this.ingest([response]);
                cb(response);
                this.off(msgId);
            };

            this._requestChannels[msgId] = this.subscribeToChannel(`topgun/@${msgId}`, cbWrap);

            return super.put({ graph, msgId, replyTo, cb: cbWrap });
        }
        else
        {
            return super.put({ graph, msgId, replyTo, cb });
        }
    }

    async authenticate(pub: string, priv: string): Promise<void>
    {
        await this.waitForConnection();
        this.doAuth(pub, priv);

        for await (const _event of this.socket.listener('connect'))
        {
            this.doAuth(pub, priv);
        }
    }

    publishToChannel(channelName: string, msg: Message): SocketConnector
    {
        this.socket.transmitPublish(channelName, msg);
        return this;
    }

    subscribeToChannel(channelName: string, cb?: MessageCb, opts?: SubscribeOptions): TGChannel<any>
    {
        const channel = this.socket.subscribe(channelName, opts);
        this.onChannelMessage(channel, cb);
        return channel;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async doAuth(pub: string, priv: string): Promise<{channel: string, data: any}>
    {
        const id        = this.socket.id;
        const timestamp = new Date().getTime();
        const challenge = `${id}/${timestamp}`;
        const proof     = await sign(challenge, { pub, priv }, { raw: true });

        return this.socket.invoke('login', { proof, pub });
    }

    private async onChannelMessage(channel: TGChannel<any>, cb?: MessageCb): Promise<void>
    {
        for await (let msg of channel)
        {
            this.ingest([msg]);
            if (cb)
            {
                cb(msg);
            }
        }
    }

    private onOutputProcessed(msg: any): void
    {
        if (msg && this.socket)
        {
            const replyTo = msg['@'];

            if (replyTo)
            {
                this.publishToChannel(`topgun/@${replyTo}`, msg);
            }
            else
            {
                if ('get' in msg)
                {
                    this.publishToChannel('topgun/get', msg);
                }
                else if ('put' in msg)
                {
                    this.publishToChannel('topgun/put', msg);
                }
            }
        }
    }

    private async onConnect(): Promise<void>
    {
        for await (const _event of this.socket.listener('connect'))
        {
            console.log(`SC client ${_event.id} is connected.`);
            try
            {
                this.events.connection.trigger(true);
            }
            catch (error)
            {
                console.error(error);
            }
        }
    }

    private async onError(): Promise<void>
    {
        for await (const _event of this.socket.listener('error'))
        {
            console.error('Socket Connection Error', _event.error.stack, _event.error.message);
            this.events.connection.trigger(false);
        }
    }
}

export function createConnector(opts: ClientOptions|undefined): SocketConnector
{
    return new SocketConnector(opts);
}

