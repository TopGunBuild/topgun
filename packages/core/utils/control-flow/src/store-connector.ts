import { EventEmitter } from "@topgunbuild/eventemitter";
import { Queue } from "./queue";

export type ConnectorState = 
    | "connecting"
    | "opened"
    | "closed"
    | "errored";

export class StoreConnector<TInbound, TOutbound, TInQueueOutput = TOutbound, TOutQueueInput = TInbound> extends EventEmitter {
    readonly name: string;
    isConnected: boolean;

    protected readonly inputQueue: Queue<TInbound, TInQueueOutput>;
    protected readonly outputQueue: Queue<TOutbound, TOutQueueInput>;

    /**
     * Constructor
     */
    protected constructor(name = 'StoreConnector')
    {
        super();
        this.isConnected = false;
        this.name        = name;

        this.inputQueue  = new Queue<TInbound, TInQueueOutput>(`${name}.inputQueue`);
        this.outputQueue = new Queue<TOutbound, TOutQueueInput>(`${name}.outputQueue`);

        this.on('stateChange', (state: ConnectorState) => {
            this.onConnectedChange(state === 'opened');
        });

        this.inputQueue.on('completed', (msg: TInQueueOutput) => {
            this.emit('receiveMessage', msg);
        });
    }

    /**
     * Wait for the connection to be established
     */
    public waitForConnection(): Promise<void>
    {
        if (this.isConnected)
        {
            return Promise.resolve();
        }

        return this.waitFor('connect');
    }

    /**
     * Queue outgoing message for sending
     *
     * @param msg The wire protocol messages to enqueue
     */
    public send(msg: TOutbound): StoreConnector<TInbound, TOutbound, TInQueueOutput, TOutQueueInput>
    {
        this.outputQueue.enqueue(msg);
        if (this.isConnected)
        {
            this.outputQueue.process();
        }
        else
        {
            this.emit('pendingOutbound', msg);
        }

        return this;
    }

    /**
     * Queue incoming message for processing
     *
     * @param msg
     */
    public ingest(msg: TInbound): StoreConnector<TInbound, TOutbound, TInQueueOutput, TOutQueueInput>
    {
        this.inputQueue.enqueue(msg).process();

        return this;
    }

    /**
     * Handles connection state changes
     *
     * @param connected
     */
    protected onConnectedChange(connected?: boolean): void
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