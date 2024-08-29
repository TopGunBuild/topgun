import { notNil } from '@topgunbuild/utils';

export class Peekable<T>
{
    private iter: Iterator<T>;
    private peeked: T|null;

    constructor(iter: Iterator<T>)
    {
        this.iter   = iter;
        this.peeked = null;
    }

    next(): T|null
    {
        if (notNil(this.peeked))
        {
            const value = this.peeked;
            this.peeked = null;
            return value;
        }
        return this.iter.next().value;
    }

    peek(): T|null
    {
        if (!notNil(this.peeked))
        {
            this.peeked = this.iter.next().value;
        }
        return this.peeked;
    }

    nextIf(func: (item: T) => boolean): T|null
    {
        const matched = this.next();
        if (notNil(matched) && func(matched))
        {
            return matched;
        }
        this.peeked = matched;
        return null;
    }
}


