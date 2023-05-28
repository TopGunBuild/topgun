import { TGGraphWireConnector } from './graph-wire-connector';
import { TGChannel } from 'topgun-socket/channel';
import {
    TGSocketClientOptions,
    TGClientSocket,
    create as createSocketClient,
    SubscribeOptions,
} from 'topgun-socket/client';
import { TGGet, TGMessage, TGMessageCb, TGPut } from '../../types';
import { generateMessageId } from '../graph/graph-utils';
import { sign } from '../../sea';

/* eslint-disable @typescript-eslint/no-empty-function */
export class TGWebSocketGraphConnector extends TGGraphWireConnector
{
    readonly socket: TGClientSocket;
    readonly opts: TGSocketClientOptions|undefined;
    readonly msgChannel?: TGChannel<any>;
    readonly getsChannel?: TGChannel<any>;
    readonly putsChannel?: TGChannel<any>;

    private readonly _requestChannels: {
        [msgId: string]: TGChannel<any>;
    };

    /**
     * Constructor
     */
    constructor(
        opts: TGSocketClientOptions|undefined,
        name = 'TGWebSocketGraphConnector',
    )
    {
        super(name);
        this._requestChannels = {};
        this.opts             = opts;
        this.socket           = createSocketClient(this.opts || {});
        this.onConnect();
        this.onError();
        this.outputQueue.completed.on(this.onOutputProcessed.bind(this));
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    off(msgId: string): TGWebSocketGraphConnector
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

    get({ soul, msgId, cb, opts }: TGGet): () => void
    {
        msgId        = msgId || generateMessageId();
        const cbWrap = (msg: any) =>
        {
            this.ingest([msg]);
            if (cb)
            {
                cb(msg);
            }
        };

        this._requestChannels[msgId] = this.subscribeToChannel(
            `topgun/nodes/${soul}`,
            cbWrap,
        );

        return super.get({ soul, msgId, cb, opts });
    }

    put({ graph, msgId = '', replyTo = '', cb }: TGPut): () => void
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

            this._requestChannels[msgId] = this.subscribeToChannel(
                `topgun/@${msgId}`,
                cbWrap,
            );

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

        // eslint-disable-next-line @typescript-eslint/no-unused-vars
        for await (const _event of this.socket.listener('connect'))
        {
            this.doAuth(pub, priv);
        }
    }

    publishToChannel(
        channelName: string,
        msg: TGMessage,
    ): TGWebSocketGraphConnector
    {
        this.socket.publish(channelName, msg);
        return this;
    }

    subscribeToChannel(
        channelName: string,
        cb?: TGMessageCb,
        opts?: SubscribeOptions,
    ): TGChannel<any>
    {
        const channel = this.socket.subscribe(channelName, opts);
        this.onChannelMessage(channel, cb);
        return channel;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async doAuth(
        pub: string,
        priv: string,
    ): Promise<{channel: string; data: any}>
    {
        const id        = this.socket.id;
        const timestamp = new Date().getTime();
        const challenge = `${id}/${timestamp}`;
        const proof     = await sign(challenge, { pub, priv }, { raw: true });

        return this.socket.invoke('login', { proof, pub });
    }

    private async onChannelMessage(
        channel: TGChannel<any>,
        cb?: TGMessageCb,
    ): Promise<void>
    {
        for await (const msg of channel)
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
            console.error(
                'Socket Connection Error',
                _event.error.stack,
                _event.error.message,
            );
            this.events.connection.trigger(false);
        }
    }
}

export function createConnector(
    opts: TGSocketClientOptions|undefined,
): TGWebSocketGraphConnector
{
    return new TGWebSocketGraphConnector(opts);
}
