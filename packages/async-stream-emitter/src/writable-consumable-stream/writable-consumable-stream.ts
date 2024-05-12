import { ConsumableStream } from '../consumable-stream/consumable-stream';
import { Consumer } from './consumer';
import { ConsumerStats } from './consumer-stats';

export class WritableConsumableStream<T> extends ConsumableStream<T>
{
    private nextConsumerId: number;
    private _consumers: Map<any, any>;
    private _tailNode: {next: null; data: {value: undefined; done: boolean}};

    /**
     * Constructor
     */
    constructor()
    {
        super();
        this.nextConsumerId = 1;
        this._consumers     = new Map();

        // Tail node of a singly linked list.
        this._tailNode = {
            next: null,
            data: {
                value: undefined,
                done : false
            }
        };
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    write(value: T): void
    {
        this._write(value, false);
    }

    close(value?: T): void
    {
        this._write(value, true);
    }

    writeToConsumer(consumerId: number, value: T): void
    {
        this._write(value, false, consumerId);
    }

    closeConsumer(consumerId: number, value?: T): void
    {
        this._write(value, true, consumerId);
    }

    kill(value?: T): void
    {
        for (const consumerId of this._consumers.keys())
        {
            this.killConsumer(consumerId, value);
        }
    }

    killConsumer(consumerId: number, value?: T): void
    {
        const consumer = this._consumers.get(consumerId);
        if (!consumer)
        {
            return;
        }
        consumer.kill(value);
    }

    getBackpressure(): number
    {
        let maxBackpressure = 0;
        for (const consumer of this._consumers.values())
        {
            const backpressure = consumer.getBackpressure();
            if (backpressure > maxBackpressure)
            {
                maxBackpressure = backpressure;
            }
        }
        return maxBackpressure;
    }

    getConsumerBackpressure(consumerId: number): number
    {
        const consumer = this._consumers.get(consumerId);
        if (consumer)
        {
            return consumer.getBackpressure();
        }
        return 0;
    }

    hasConsumer(consumerId: number): boolean
    {
        return this._consumers.has(consumerId);
    }

    setConsumer(consumerId: number, consumer: Consumer<T>): void
    {
        this._consumers.set(consumerId, consumer);
        if (!consumer.currentNode)
        {
            consumer.currentNode = this._tailNode;
        }
    }

    removeConsumer(consumerId: number): boolean
    {
        return this._consumers.delete(consumerId);
    }

    getConsumerStats(consumerId: number): ConsumerStats
    {
        const consumer = this._consumers.get(consumerId);
        if (consumer)
        {
            return consumer.getStats();
        }
        return undefined;
    }

    getConsumerStatsList(): ConsumerStats[]
    {
        const consumerStats = [];
        for (const consumer of this._consumers.values())
        {
            consumerStats.push(consumer.getStats());
        }
        return consumerStats;
    }

    createConsumer(timeout?: number): Consumer<T>
    {
        return new Consumer(this, this.nextConsumerId++, this._tailNode, timeout);
    }

    getConsumerList()
    {
        return [...this._consumers.values()];
    }

    getConsumerCount(): number
    {
        return this._consumers.size;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _write(value: T, done: boolean, consumerId?: number)
    {
        const dataNode: any = {
            data: { value, done },
            next: null
        };
        if (consumerId)
        {
            dataNode.consumerId = consumerId;
        }
        this._tailNode.next = dataNode;
        this._tailNode      = dataNode;

        for (const consumer of this._consumers.values())
        {
            consumer.write(dataNode.data);
        }
    }
}