import { ConsumerStats } from './consumer-stats';
import { ConsumerNode } from './consumer-node';
import { WritableConsumableStream } from './writable-consumable-stream';

export class Consumer<T>
{
    readonly id: number;
    readonly timeout: number;
    currentNode: ConsumerNode<T>;
    stream: WritableConsumableStream<T>;

    private _timeoutId: any;
    private _backpressure: number;
    private _isIterating: boolean;
    private _resolve: any;
    private _killPacket: {value: any; done: boolean};

    /**
     * Constructor
     */
    constructor(stream, id, startNode, timeout)
    {
        this.id            = id;
        this._backpressure = 0;
        this.stream        = stream;
        this.currentNode   = startNode;
        this.timeout       = timeout;
        this._isIterating  = false;
        this.stream.setConsumer(this.id, this);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    getStats(): ConsumerStats
    {
        const stats: ConsumerStats = {
            id          : this.id,
            backpressure: this._backpressure
        };
        if (this.timeout != null)
        {
            stats.timeout = this.timeout;
        }
        return stats;
    }

    resetBackpressure(): void
    {
        this._backpressure = 0;
    }

    applyBackpressure(packet?: any): void
    {
        this._backpressure++;
    }

    releaseBackpressure(packet?: any): void
    {
        this._backpressure--;
    }

    getBackpressure(): number
    {
        return this._backpressure;
    }

    write(packet?: any): void
    {
        if (this._timeoutId !== undefined)
        {
            clearTimeout(this._timeoutId);
            delete this._timeoutId;
        }
        this.applyBackpressure(packet);
        if (this._resolve)
        {
            this._resolve();
            delete this._resolve;
        }
    }

    kill(value?: any): void
    {
        if (this._timeoutId !== undefined)
        {
            clearTimeout(this._timeoutId);
            delete this._timeoutId;
        }
        if (this._isIterating)
        {
            this._killPacket = { value, done: true };
            this.applyBackpressure(this._killPacket);
        }
        else
        {
            this.stream.removeConsumer(this.id);
            this.resetBackpressure();
        }
        if (this._resolve)
        {
            this._resolve();
            delete this._resolve;
        }
    }

    async next(): Promise<IteratorResult<T>>
    {
        this._isIterating = true;
        this.stream.setConsumer(this.id, this);

        while (true)
        {
            if (!this.currentNode.next)
            {
                try
                {
                    await this._waitForNextItem(this.timeout);
                }
                catch (error)
                {
                    this._isIterating = false;
                    this.stream.removeConsumer(this.id);
                    throw error;
                }
            }
            if (this._killPacket)
            {
                this._isIterating = false;
                this.stream.removeConsumer(this.id);
                this.resetBackpressure();
                const killPacket = this._killPacket;
                delete this._killPacket;

                return killPacket;
            }

            this.currentNode = this.currentNode.next;
            this.releaseBackpressure(this.currentNode.data);

            if (this.currentNode.consumerId && this.currentNode.consumerId !== this.id)
            {
                continue;
            }

            if (this.currentNode.data.done)
            {
                this._isIterating = false;
                this.stream.removeConsumer(this.id);
            }

            return this.currentNode.data;
        }
    }

    return(): object
    {
        delete this.currentNode;
        this._isIterating = false;
        this.stream.removeConsumer(this.id);
        this.resetBackpressure();
        return {};
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async _waitForNextItem(timeout: number): Promise<any>
    {
        return new Promise((resolve, reject) =>
        {
            this._resolve = resolve;
            let timeoutId;
            if (timeout !== undefined)
            {
                // Create the error object in the outer scope in order
                // to get the full stack trace.
                const error = new Error('Stream consumer iteration timed out');
                (async () =>
                {
                    const delay = wait(timeout);
                    timeoutId = delay.timeoutId;
                    await delay.promise;
                    error.name = 'TimeoutError';
                    delete this._resolve;
                    reject(error);
                })();
            }
            this._timeoutId = timeoutId;
        });
    }
}

function wait(timeout)
{
    let timeoutId;
    const promise = new Promise((resolve) =>
    {
        timeoutId = setTimeout(resolve, timeout);
    });
    return { timeoutId, promise };
}
