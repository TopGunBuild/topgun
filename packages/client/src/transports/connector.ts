import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { Message } from '@topgunbuild/transport';
import { Queue } from '../control-flow/queue';

export abstract class Connector extends AsyncStreamEmitter<any>
{
    readonly name: string;
    isConnected: boolean;

    protected readonly inputQueue: Queue<Message>;
    protected readonly outputQueue: Queue<Message>;

    /**
     * Constructor
     */
    protected constructor(name = 'GraphConnector')
    {
        super();
        this.isConnected = false;
        this.name        = name;
        this.inputQueue  = new Queue<Message>(`${name}.inputQueue`);
        this.outputQueue = new Queue<Message>(`${name}.outputQueue`);

        (async () =>
        {
            for await (const _ of this.listener('connect'))
            {
                this.#onConnectedChange(true);
            }
        })();

        (async () =>
        {
            for await (const _ of this.listener('disconnect'))
            {
                this.#onConnectedChange(false);
            }
        })();
    }

    off(_msgId: string): Connector
    {
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

    async disconnect(): Promise<void>
    {
    }

    send(message: Message): Connector
    {
        this.outputQueue.enqueue(message);
        if (this.isConnected)
        {
            this.outputQueue.process();
        }

        return this;
    }

    ingest(message: Message): Connector
    {
        this.inputQueue.enqueue(message).process();

        return this;
    }

    #onConnectedChange(connected?: boolean): void
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
