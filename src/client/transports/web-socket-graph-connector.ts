import { TGGraphWireConnector } from './graph-wire-connector';
import { TGChannel } from 'topgun-socket/channel';
import {
    TGSocketClientOptions,
    TGClientSocket,
    create as createSocketClient,
    SubscribeOptions,
} from 'topgun-socket/client';
import { TGGet, TGMessage, TGMessageCb, TGPut } from '../../types';
import { sign } from '../../sea';
import { uuidv4 } from '../../utils/uuidv4';

/* eslint-disable @typescript-eslint/no-empty-function */
/* eslint-disable-next-line @typescript-eslint/no-unused-vars */
export class TGWebSocketGraphConnector extends TGGraphWireConnector
{
    readonly client: TGClientSocket;
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
        this.client           = createSocketClient(this.opts || {});
        this.onConnect();
        this.onError();
        this.outputQueue.completed.on(this.onOutputProcessed.bind(this));
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async disconnect(): Promise<void>
    {
        const cleanupTasks = [];

        if (this.client)
        {
            if (this.client.state !== this.client.CLOSED)
            {
                cleanupTasks.push(
                    Promise.race([
                        this.client.listener('disconnect').once(),
                        this.client.listener('connectAbort').once()
                    ])
                );
                this.client.disconnect();
            }
            else
            {
                this.client.disconnect();
            }
        }

        await Promise.all(cleanupTasks);
    }

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

    get({ msgId, cb, options }: TGGet): () => void
    {
        const soul   = options['#'];
        msgId        = msgId || uuidv4();
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
            {
                data: options
            }
        );

        return super.get({ msgId, cb, options });
    }

    put({ graph, msgId = '', replyTo = '', cb }: TGPut): () => void
    {
        if (!graph)
        {
            return () =>
            {
            };
        }

        msgId = msgId || uuidv4();

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

        for await (const _event of this.client.listener('connect'))
        {
            this.doAuth(pub, priv);
        }
    }

    publishToChannel(
        channelName: string,
        msg: TGMessage,
    ): TGWebSocketGraphConnector
    {
        const messageId = msg['#'];
        const channel   = this._requestChannels[messageId];

        if (channel)
        {
            channel
                .listener('subscribe')
                .once()
                .then(() =>
                {
                    this.client.publish(channelName, msg);
                });
        }
        else
        {
            this.client.publish(channelName, msg);
        }

        return this;
    }

    subscribeToChannel(
        channelName: string,
        cb?: TGMessageCb,
        opts?: SubscribeOptions,
    ): TGChannel<any>
    {
        const channel = this.client.subscribe(channelName, opts);
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
        const id        = this.client.id;
        const timestamp = new Date().getTime();
        const challenge = `${id}/${timestamp}`;
        const proof     = await sign(challenge, { pub, priv }, { raw: true });

        return this.client.invoke('login', { proof, pub });
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

    private onOutputProcessed(msg: TGMessage): void
    {
        if (msg && this.client)
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
        for await (const _event of this.client.listener('connect'))
        {
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
        for await (const _event of this.client.listener('error'))
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
