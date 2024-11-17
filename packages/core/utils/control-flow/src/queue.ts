import { MiddlewareSystem } from './middleware-system';
import { EventEmitter } from '@topgunbuild/eventemitter';

export class Queue<TInput, TOutput = TInput> extends EventEmitter
{
    isProcessing: boolean;
    readonly middleware: MiddlewareSystem<TInput, TOutput>;
    private _queue: TInput[];

    constructor(name = 'ProcessQueue')
    {
        super();
        this.isProcessing = false;
        this.middleware   = new MiddlewareSystem<TInput, TOutput>(`${name}.middleware`);
        this._queue       = [];
    }

    enqueue(item: TInput): Queue<TInput, TOutput>
    {
        this._queue.splice(0, 0, item);
        return this;
    }

    dequeue(): TInput|undefined
    {
        return this._queue.pop();
    }

    count(): number
    {
        return this._queue.length;
    }

    async processNext(): Promise<void>
    {
        let item: TInput|TOutput|undefined = this.dequeue();

        if (!item)
        {
            return;
        }

        item = await this.middleware.process(item) as TOutput|undefined;

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
