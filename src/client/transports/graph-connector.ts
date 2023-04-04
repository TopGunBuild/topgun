import { TGGet, TGPut, TGGraphData, TGMessage } from '../../types'
import { TGEvent } from '../control-flow/event'
import { TGProcessQueue } from '../control-flow/process-queue'
import { TGGraph } from '../graph/graph'

export abstract class TGGraphConnector
{
    public readonly name: string;
    public isConnected: boolean;

    public readonly events: {
        readonly graphData: TGEvent<TGGraphData,
            string|undefined,
            string|undefined>
        readonly receiveMessage: TGEvent<TGMessage>
        readonly connection: TGEvent<boolean>
    };

    protected readonly inputQueue: TGProcessQueue<TGMessage>;
    protected readonly outputQueue: TGProcessQueue<TGMessage>;

    constructor(name = 'GraphConnector')
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
            receiveMessage: new TGEvent<TGMessage>(`${name}.events.receiveMessage`)
        };

        this.__onConnectedChange = this.__onConnectedChange.bind(this);
        this.events.connection.on(this.__onConnectedChange)
    }

    public connectToGraph(graph: TGGraph): TGGraphConnector
    {
        graph.events.off.on(this.off);
        return this
    }

    public off(_msgId: string): TGGraphConnector
    {
        return this
    }

    public sendPutsFromGraph(graph: TGGraph): TGGraphConnector
    {
        graph.events.put.on(this.put);
        return this
    }

    public sendRequestsFromGraph(graph: TGGraph): TGGraphConnector
    {
        graph.events.get.on(req =>
        {
            this.get(req);
        });
        return this
    }

    public waitForConnection(): Promise<void>
    {
        if (this.isConnected)
        {
            return Promise.resolve()
        }
        return new Promise(ok =>
        {
            const onConnected = (connected?: boolean) =>
            {
                if (!connected)
                {
                    return
                }
                ok();
                this.events.connection.off(onConnected)
            };
            this.events.connection.on(onConnected)
        })
    }

    /**
     * Send graph data for one or more nodes
     *
     * @returns A function to be called to clean up callback listeners
     */
    public put(_params: TGPut): () => void
    {
        return () =>
        {
        }
    }

    /**
     * Request data for a given soul
     *
     * @returns A function to be called to clean up callback listeners
     */
    public get(_params: TGGet): () => void
    {
        return () =>
        {
        }
    }

    /**
     * Queues outgoing messages for sending
     *
     * @param msgs The wire protocol messages to enqueue
     */
    public send(msgs: readonly TGMessage[]): TGGraphConnector
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
    public ingest(msgs: readonly TGMessage[]): TGGraphConnector
    {
        this.inputQueue.enqueueMany(msgs).process();

        return this
    }

    private __onConnectedChange(connected?: boolean): void
    {
        if (connected)
        {
            this.isConnected = true;
            this.outputQueue.process();
        }
        else
        {
            this.isConnected = false
        }
    }
}
