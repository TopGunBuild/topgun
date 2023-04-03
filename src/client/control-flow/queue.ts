import { Message } from '../../types'

export class Queue<T = Message>
{
    public readonly name: string;
    private _queue: T[];

    constructor(name = 'Queue')
    {
        this.name   = name;
        this._queue = []
    }

    public count(): number
    {
        return this._queue.length
    }

    public has(item: T): boolean
    {
        return this._queue.indexOf(item) !== -1
    }

    public enqueue(item: T): Queue<T>
    {
        if (this.has(item))
        {
            return this
        }

        this._queue.splice(0, 0, item);
        return this
    }

    public dequeue(): T|undefined
    {
        return this._queue.pop()
    }

    public enqueueMany(items: readonly T[]): Queue<T>
    {
        const filtered = items.filter(item => !this.has(item));

        if (filtered.length)
        {
            this._queue.splice(0, 0, ...filtered.slice().reverse())
        }

        return this
    }
}
