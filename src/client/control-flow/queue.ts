import { TGMessage } from '../../types';

export class TGQueue<T = TGMessage> 
{
    readonly name: string;
    private _queue: T[];

    /**
     * Constructor
     */
    constructor(name = 'Queue') 
    {
        this.name = name;
        this._queue = [];
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    count(): number 
    {
        return this._queue.length;
    }

    has(item: T): boolean 
    {
        return this._queue.indexOf(item) !== -1;
    }

    enqueue(item: T): TGQueue<T> 
    {
        if (this.has(item)) 
        {
            return this;
        }

        this._queue.splice(0, 0, item);
        return this;
    }

    dequeue(): T | undefined 
    {
        return this._queue.pop();
    }

    enqueueMany(items: readonly T[]): TGQueue<T> 
    {
        const filtered = items.filter(item => !this.has(item));

        if (filtered.length) 
        {
            this._queue.splice(0, 0, ...filtered.slice().reverse());
        }

        return this;
    }
}
