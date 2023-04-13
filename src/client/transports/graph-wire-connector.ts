import { TGGet, TGPut, TGMessage, TGMessageCb } from '../../types';
import { generateMessageId } from '../graph/graph-utils';
import { TGGraphConnector } from './graph-connector';

/* eslint-disable @typescript-eslint/no-empty-function */
export class TGGraphWireConnector extends TGGraphConnector 
{
    private readonly _callbacks: {
        [msgId: string]: TGMessageCb;
    };

    /**
     * Constructor
     */
    constructor(name = 'GraphWireConnector') 
    {
        super(name);
        this._callbacks = {};

        this._onProcessedInput = this._onProcessedInput.bind(this);
        this.inputQueue.completed.on(this._onProcessedInput);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    off(msgId: string): TGGraphWireConnector 
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
    put({ graph, msgId = '', replyTo = '', cb }: TGPut): () => void 
    {
        if (!graph) 
        {
            return () => 
            {};
        }
        const msg: TGMessage = {
            put: graph,
        };
        if (msgId) 
        {
            msg['#'] = msgId;
        }
        if (replyTo) 
        {
            msg['@'] = replyTo;
        }

        return this.req(msg, cb);
    }

    /**
     * Request data for a given soul
     *
     * @returns A function to be called to clean up callback listeners
     */
    get({ soul, cb, msgId = '' }: TGGet): () => void 
    {
        const get = { '#': soul };
        const msg: TGMessage = { get };
        if (msgId) 
        {
            msg['#'] = msgId;
        }

        return this.req(msg, cb);
    }

    /**
     * Send a message that expects responses via @
     *
     * @param msg
     * @param cb
     */
    req(msg: TGMessage, cb?: TGMessageCb): () => void 
    {
        const reqId = (msg['#'] = msg['#'] || generateMessageId());
        if (cb) 
        {
            this._callbacks[reqId] = cb;
        }
        this.send([msg]);
        return () => 
        {
            this.off(reqId);
        };
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _onProcessedInput(msg?: TGMessage): void 
    {
        if (!msg) 
        {
            return;
        }
        const id = msg['#'];
        const replyTo = msg['@'];

        if (msg.put) 
        {
            this.events.graphData.trigger(msg.put, id, replyTo);
        }

        if (replyTo) 
        {
            const cb = this._callbacks[replyTo];
            if (cb) 
            {
                cb(msg);
            }
        }

        this.events.receiveMessage.trigger(msg);
    }
}
