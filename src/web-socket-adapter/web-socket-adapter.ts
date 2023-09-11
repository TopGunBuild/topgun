import {
    TGSocketClientOptions,
    TGClientSocket,
    create as createSocketClient,
    SubscribeOptions,
} from '@topgunbuild/socket/client';
import { TGChannel } from '@topgunbuild/socket/channel';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { TGGraphAdapter, TGGraphData, TGMessage, TGMessageCb, TGOptionsGet } from '../types';
import { uuidv4 } from '../utils';
import { TGChangeSetEntry } from '../federation-adapter';

export class WebSocketAdapter extends AsyncStreamEmitter<any> implements TGGraphAdapter
{
    readonly client: TGClientSocket;
    readonly opts: TGSocketClientOptions|undefined;

    private readonly _requestChannels: {
        [msgId: string]: TGChannel<any>;
    };

    /**
     * Constructor
     */
    constructor(opts: TGSocketClientOptions|undefined)
    {
        super();
        this._requestChannels = {};
        this.opts             = opts;
        this.client           = createSocketClient(this.opts || {});
        this.#onConnect();
        this.#onError();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    put(graphData: TGGraphData): Promise<TGGraphData|null>
    {
        return new Promise<TGGraphData|null>((resolve) =>
        {
            const msgId                  = uuidv4();
            const cb                     = (res: TGGraphData|null) =>
            {
                this.#off(msgId);
                resolve(res);
            };
            this._requestChannels[msgId] = this.#subscribeToChannel(`topgun/@${msgId}`, cb);

            this.#publishToChannel('topgun/put', {
                '#'  : msgId,
                'put': graphData
            });
        });
    }

    get(opts: TGOptionsGet): Promise<TGGraphData>
    {
        return new Promise<TGGraphData>((resolve) =>
        {
            const soul                   = opts['#'];
            const msgId                  = uuidv4();
            const cb                     = (res: TGMessage) =>
            {
                this.#off(msgId);
                resolve(res.put);
            };
            this._requestChannels[msgId] = this.#subscribeToChannel(
                `topgun/nodes/${soul}`,
                cb,
                {
                    data: opts
                }
            );
            this.#publishToChannel('topgun/get', {
                '#'  : msgId,
                'get': opts
            });
        });
    }

    onChange(handler: (change: TGChangeSetEntry) => void, from?: string): () => void
    {
        return () =>
        {

        };
    }

    close(): void
    {
        this.client.disconnect();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #off(msgId: string): void
    {
        const channel = this._requestChannels[msgId];

        if (channel)
        {
            channel.unsubscribe();
            delete this._requestChannels[msgId];
        }
    }

    #publishToChannel(channelName: string, msg: TGMessage): void
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
    }

    #subscribeToChannel(channelName: string, cb: TGMessageCb, opts?: SubscribeOptions): TGChannel<any>
    {
        const channel = this.client.subscribe(channelName, opts);
        this.#onChannelMessage(channel, cb);
        return channel;
    }

    async #onChannelMessage(channel: TGChannel<any>, cb: TGMessageCb,): Promise<void>
    {
        try
        {
            for await (const msg of channel)
            {
                cb(msg);
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
}
