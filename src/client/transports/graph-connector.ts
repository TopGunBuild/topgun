import { AsyncStreamEmitter } from 'topgun-async-stream-emitter';
import { TGGet, TGPut, TGMessage } from '../../types';
import { TGProcessQueue } from '../control-flow/process-queue';
import { TGGraph } from '../graph/graph';

/* eslint-disable @typescript-eslint/no-unused-vars */
/* eslint-disable @typescript-eslint/no-empty-function */
export abstract class TGGraphConnector extends AsyncStreamEmitter<any>
{
    readonly name: string;
    isConnected: boolean;

    protected readonly inputQueue: TGProcessQueue<TGMessage>;
    protected readonly outputQueue: TGProcessQueue<TGMessage>;

    /**
     * Constructor
     */
    protected constructor(name = 'GraphConnector')
    {
        super();
        this.isConnected = false;
        this.name        = name;

        this.put = this.put.bind(this);
        this.off = this.off.bind(this);

        this.inputQueue  = new TGProcessQueue<TGMessage>(`${name}.inputQueue`);
        this.outputQueue = new TGProcessQueue<TGMessage>(`${name}.outputQueue`);

        (async () =>
        {
            for await (const value of this.listener('connect'))
            {
                this._onConnectedChange(true);
            }
        })();

        (async () =>
        {
            for await (const value of this.listener('disconnect'))
            {
                this._onConnectedChange(false);
            }
        })();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    connectToGraph(graph: TGGraph): TGGraphConnector
    {
        (async () =>
        {
            for await (const value of graph.listener('off'))
            {
                this.off(value);
            }
        })();
        return this;
    }

    off(_msgId: string): TGGraphConnector
    {
        return this;
    }

    sendPutsFromGraph(graph: TGGraph): TGGraphConnector
    {
        (async () =>
        {
            for await (const value of graph.listener('put'))
            {
                this.put(value);
            }
        })();
        return this;
    }

    sendRequestsFromGraph(graph: TGGraph): TGGraphConnector
    {
        (async () =>
        {
            for await (const value of graph.listener('get'))
            {
                this.get(value);
            }
        })();
        return this;
    }

    waitForConnection(): Promise<void>
    {
        if (this.isConnected)
        {
            return Promise.resolve();
        }

        return this.listener('connect').once();
    }

    /**
     * Send graph data for one or more nodes
     *
     * @returns A function to be called to clean up callback listeners
     */
    put(_params: TGPut): () => void
    {
        return () =>
        {
        };
    }

    /**
     * Request data for a given soul
     *
     * @returns A function to be called to clean up callback listeners
     */
    get(_params: TGGet): () => void
    {
        return () =>
        {
        };
    }

    /**
     * Queues outgoing messages for sending
     *
     * @param msgs The wire protocol messages to enqueue
     */
    send(msgs: readonly TGMessage[]): TGGraphConnector
    {
        this.outputQueue.enqueueMany(msgs);
        if (this.isConnected)
        {
            this.outputQueue.process();
        }

        return this;
    }

    /**
     * Queue incoming messages for processing
     *
     * @param msgs
     */
    ingest(msgs: readonly TGMessage[]): TGGraphConnector
    {
        this.inputQueue.enqueueMany(msgs).process();

        return this;
    }

    async disconnect(): Promise<void>
    {
    }

    async authenticate(pub: string, priv: string): Promise<void>
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _onConnectedChange(connected?: boolean): void
    {
        if (connected)
        {
            this.isConnected = true;
            this.outputQueue.process();
        }
        else
        {
            this.isConnected = false;
        }
    }
}
