import { TGGet, TGPut, TGGraphData, TGMessage } from '../../types';
import { TGEvent } from '../control-flow/event';
import { TGProcessQueue } from '../control-flow/process-queue';
import { TGGraph } from '../graph/graph';

/* eslint-disable @typescript-eslint/no-unused-vars */
/* eslint-disable @typescript-eslint/no-empty-function */
export abstract class TGGraphConnector
{
    readonly name: string;
    isConnected: boolean;

    readonly events: {
        readonly graphData: TGEvent<TGGraphData,
        string|undefined,
        string|undefined>;
        readonly receiveMessage: TGEvent<TGMessage>;
        readonly connection: TGEvent<boolean>;
    };

    protected readonly inputQueue: TGProcessQueue<TGMessage>;
    protected readonly outputQueue: TGProcessQueue<TGMessage>;

    /**
     * Constructor
     */
    protected constructor(name = 'GraphConnector')
    {
        this.isConnected = false;
        this.name        = name;

        this.put = this.put.bind(this);
        this.off = this.off.bind(this);

        this.inputQueue  = new TGProcessQueue<TGMessage>(`${name}.inputQueue`);
        this.outputQueue = new TGProcessQueue<TGMessage>(`${name}.outputQueue`);

        this.events = {
            connection    : new TGEvent(`${name}.events.connection`),
            graphData     : new TGEvent<TGGraphData>(`${name}.events.graphData`),
            receiveMessage: new TGEvent<TGMessage>(
                `${name}.events.receiveMessage`,
            ),
        };

        this._onConnectedChange = this._onConnectedChange.bind(this);
        this.events.connection.on(this._onConnectedChange);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    connectToGraph(graph: TGGraph): TGGraphConnector
    {
        graph.events.off.on(this.off);
        return this;
    }

    off(_msgId: string): TGGraphConnector
    {
        return this;
    }

    sendPutsFromGraph(graph: TGGraph): TGGraphConnector
    {
        graph.events.put.on(this.put);
        return this;
    }

    sendRequestsFromGraph(graph: TGGraph): TGGraphConnector
    {
        graph.events.get.on((req) =>
        {
            this.get(req);
        });
        return this;
    }

    waitForConnection(): Promise<void>
    {
        if (this.isConnected)
        {
            return Promise.resolve();
        }
        return new Promise((ok) =>
        {
            const onConnected = (connected?: boolean) =>
            {
                if (!connected)
                {
                    return;
                }
                ok();
                this.events.connection.off(onConnected);
            };
            this.events.connection.on(onConnected);
        });
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

    disconnect(): void
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
