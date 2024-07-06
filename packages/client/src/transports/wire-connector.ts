import { Message } from '@topgunbuild/transport';
import { Connector } from './connector';
import { ConnectorSendOptions, MessageCb } from '../types';

export class WireConnector extends Connector
{
    private readonly _callbacks: {
        [msgId: string]: MessageCb;
    };

    /**
     * Constructor
     */
    constructor(name = 'WireConnector')
    {
        super(name);
        this._callbacks = {};

        (async () =>
        {
            for await (const value of this.inputQueue.listener('completed'))
            {
                this.#onProcessedInput(value);
            }
        })();
    }

    off(msgId: string): WireConnector
    {
        super.off(msgId);
        delete this._callbacks[msgId];
        return this;
    }

    async disconnect(): Promise<void>
    {
        Object.keys(this._callbacks).forEach(msgId => this.off(msgId));
        return super.disconnect();
    }

    send(message: Message, options: ConnectorSendOptions): () => void
    {
        if (options?.cb)
        {
            this._callbacks[message.idString] = options.cb;
        }
        return super.send(message, options);
    }

    #onProcessedInput(msg: Message): void
    {
        const replyToId = msg.replyToIdString;

        if (replyToId)
        {
            const cb = this._callbacks[replyToId];
            if (cb)
            {
                cb(msg);
            }
        }

        this.emit('receiveMessage', msg);
    }
}
