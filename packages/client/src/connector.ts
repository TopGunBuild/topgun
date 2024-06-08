import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { TransportRecord } from '@topgunbuild/store';
import { Queue } from './control-flow/queue';
import { PublicKey } from '@topgunbuild/crypto';

export abstract class Connector extends AsyncStreamEmitter<any>
{
    readonly name: string;
    isConnected: boolean;

    protected readonly inputQueue: Queue<TransportRecord>;
    protected readonly outputQueue: Queue<TransportRecord>;

    /**
     * Constructor
     */
    protected constructor(name = 'GraphConnector')
    {
        super();
        this.isConnected = false;
        this.name        = name;
        this.inputQueue  = new Queue<TransportRecord>(`${name}.inputQueue`);
        this.outputQueue = new Queue<TransportRecord>(`${name}.outputQueue`);

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

    publishMessage(from: PublicKey, message: any): () => void
    {
        return () =>
        {
        };
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
