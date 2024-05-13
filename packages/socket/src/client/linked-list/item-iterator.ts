import { Item } from './item';

/**
 * Creates an iterator that iterates over a list (through an item).
 *
 * @template {Item} [T=Item]
 */
export class ItemIterator<T extends Item>
{
    item: T | null = null;

    /**
     * Create a new iterator.
     */
    constructor(item: T | null)
    {
        this.item = item;
    }

    /**
     * Move to the next item.
     */
    next(): IteratorResult<T | null>
    {
        const value = this.item;

        if (value)
        {
            this.item = value.next as T;
            return { value, done: false };
        }

        return { value: null, done: true };
    }
}
