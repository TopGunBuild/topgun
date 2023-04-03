import { Get, Put, Message, MessageCb } from '../../types'
import { generateMessageId } from '../graph/graph-utils'
import { GraphConnector } from './graph-connector'

export class GraphWireConnector extends GraphConnector
{
    private readonly _callbacks: {
         [msgId: string]: MessageCb
    };

    constructor(name = 'GraphWireConnector')
    {
        super(name);
        this._callbacks = {};

        this._onProcessedInput = this._onProcessedInput.bind(this);
        this.inputQueue.completed.on(this._onProcessedInput)
    }

    public off(msgId: string): GraphWireConnector
    {
        super.off(msgId);
        delete this._callbacks[msgId];
        return this;
    }

    /**
     * Send graph data for one or more nodes
     *
     * @returns A function to be called to clean up callback listeners
     */
    public put({ graph, msgId = '', replyTo = '', cb }: Put): () => void
    {
        if (!graph)
        {
            return () =>
            {
            }
        }
        const msg: Message = {
            put: graph
        };
        if (msgId)
        {
            msg['#'] = msgId
        }
        if (replyTo)
        {
            msg['@'] = replyTo
        }

        return this.req(msg, cb);
    }

    /**
     * Request data for a given soul
     *
     * @returns A function to be called to clean up callback listeners
     */
    public get({ soul, cb, msgId = '' }: Get): () => void
    {
        const get          = { '#': soul };
        const msg: Message = { get };
        if (msgId)
        {
            msg['#'] = msgId
        }

        return this.req(msg, cb)
    }

    /**
     * Send a message that expects responses via @
     *
     * @param msg
     * @param cb
     */
    public req(msg: Message, cb?: MessageCb): () => void
    {
        const reqId = (msg['#'] = msg['#'] || generateMessageId());
        if (cb)
        {
            this._callbacks[reqId] = cb
        }
        this.send([msg]);
        return () =>
        {
            this.off(reqId);
        }
    }

    private _onProcessedInput(msg?: Message): void
    {
        if (!msg)
        {
            return
        }
        const id      = msg['#'];
        const replyTo = msg['@'];

        if (msg.put)
        {
            this.events.graphData.trigger(msg.put, id, replyTo)
        }

        if (replyTo)
        {
            const cb = this._callbacks[replyTo];
            if (cb)
            {
                cb(msg)
            }
        }

        this.events.receiveMessage.trigger(msg)
    }
}
