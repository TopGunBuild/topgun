import { Message } from '@topgunbuild/transport';
import { Connector } from './connector';
import { MessageCb } from '../types';

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

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    off(msgId: string): WireConnector
    {
        super.off(msgId);
        delete this._callbacks[msgId];
        return this;
    }

    sendMessage(message: Message, cb?: MessageCb): () => void
    {
        const reqId = message.idString;
        if (cb)
        {
            this._callbacks[reqId] = cb;
        }
        this.send(message);
        return () =>
        {
            this.off(reqId);
        };
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #onProcessedInput(msg: Message): void
    {
        const id        = msg.idString;
        const replyToId = msg.replyToIdString;

        // if (msg.put)
        // {
        //     this.emit('graphData', {
        //         data: msg.put,
        //         id,
        //         replyToId,
        //     });
        // }

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
