import { MiddlewareSystem } from './middleware-system';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';

export class Queue<T> extends AsyncStreamEmitter<any>
{
    isProcessing: boolean;
    readonly middleware: MiddlewareSystem<T>;
    private _queue: T[];

    constructor(name = 'ProcessQueue')
    {
        super();
        this.isProcessing = false;
        this.middleware   = new MiddlewareSystem<T>(`${name}.middleware`);
        this._queue       = [];
    }

    enqueue(item: T): Queue<T>
    {
        this._queue.splice(0, 0, item);
        return this;
    }

    dequeue(): T|undefined
    {
        return this._queue.pop();
    }

    count(): number
    {
        return this._queue.length;
    }

    async processNext(): Promise<void>
    {
        let item = this.dequeue();

        if (!item)
        {
            return;
        }

        item = (await this.middleware.process(item)) as T|undefined;

        if (item)
        {
            this.emit('completed', item);
        }
    }

    async process(): Promise<void>
    {
        if (this.isProcessing)
        {
            return;
        }

        if (!this.count())
        {
            return;
        }

        this.isProcessing = true;
        while (this.count())
        {
            try
            {
                await this.processNext();
            }
            catch (e)
            {
                console.error('Process Queue error', e);
            }
        }

        this.emit('emptied', true);
        this.isProcessing = false;
    }
}
