import { Get, Put, GraphData, Message } from '../../types'
import { Event } from '../control-flow/event'
import { ProcessQueue } from '../control-flow/process-queue'
import { Graph } from '../graph/graph'

export abstract class GraphConnector
{
    public readonly name: string;
    public isConnected: boolean;

    public readonly events: {
        readonly graphData: Event<GraphData,
            string|undefined,
            string|undefined>
        readonly receiveMessage: Event<Message>
        readonly connection: Event<boolean>
    };

    protected readonly inputQueue: ProcessQueue<Message>;
    protected readonly outputQueue: ProcessQueue<Message>;

    constructor(name = 'GraphConnector')
    {
        this.isConnected = false;
        this.name        = name;

        this.put = this.put.bind(this);
        this.off = this.off.bind(this);

        this.inputQueue  = new ProcessQueue<Message>(`${name}.inputQueue`);
        this.outputQueue = new ProcessQueue<Message>(`${name}.outputQueue`);

        this.events = {
            connection    : new Event(`${name}.events.connection`),
            graphData     : new Event<GraphData>(`${name}.events.graphData`),
            receiveMessage: new Event<Message>(`${name}.events.receiveMessage`)
        };

        this.__onConnectedChange = this.__onConnectedChange.bind(this);
        this.events.connection.on(this.__onConnectedChange)
    }

    public connectToGraph(graph: Graph): GraphConnector
    {
        graph.events.off.on(this.off);
        return this
    }

    public off(_msgId: string): GraphConnector
    {
        return this
    }

    public sendPutsFromGraph(graph: Graph): GraphConnector
    {
        graph.events.put.on(this.put);
        return this
    }

    public sendRequestsFromGraph(graph: Graph): GraphConnector
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
    public put(_params: Put): () => void
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
    public get(_params: Get): () => void
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
    public send(msgs: readonly Message[]): GraphConnector
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
    public ingest(msgs: readonly Message[]): GraphConnector
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
