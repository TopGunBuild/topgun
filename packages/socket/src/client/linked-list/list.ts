import { appendAll } from './append-all';
import { Item } from './item';
import { ItemIterator } from './item-iterator';

/**
 * Double linked list.
 */
export class LinkedList<T extends Item> implements Iterable<T>
{
    size: number;
    head: T | null;
    tail: T | null;

    /**
     * Create a new `this` from the given array of items.
     *
     * Ignores `null` or `undefined` values.
     * Throws an error when a given item has no `detach`, `append`, or `prepend`
     * methods.
     */
    static from<T extends Item>(items: Array<T | null | undefined>)
    {
        const list = new this();
        return appendAll(list, items);
    }

    /**
     * Create a new `this` from the given arguments.
     *
     * Ignores `null` or `undefined` values.
     * Throws an error when a given item has no `detach`, `append`, or `prepend`
     * methods.
     */
    static of<T extends Item>(
        ...items: Array<T | null | undefined>
    ): LinkedList<T>
    {
        const list = new this() as LinkedList<T>;
        return appendAll<T>(list, items);
    }

    /**
     * Create a new list from the given items.
     *
     * Ignores `null` or `undefined` values.
     * Throws an error when a given item has no `detach`, `append`, or `prepend`
     * methods.
     */
    constructor(...items: Array<T | null | undefined>)
    {
        this.size = 0;
        this.tail = null;
        this.head = null;
        appendAll(this, items);
    }

    /**
     * Append an item to a list.
     *
     * Throws an error when the given item has no `detach`, `append`, or `prepend`
     * methods.
     * Returns the given item.
     */
    append(item: T | null | undefined): T | false
    {
        if (!item)
        {
            return false;
        }

        if (!item.append || !item.prepend || !item.detach)
        {
            throw new Error(
                'An argument without append, prepend, or detach methods was given to `List#append`.'
            );
        }

        // If self has a last item, defer appending to the last items append method,
        // and return the result.
        if (this.tail)
        {
            return this.tail.append(item) as T;
        }

        // If self has a first item, defer appending to the first items append method,
        // and return the result.
        if (this.head)
        {
            return this.head.append(item) as T;
        }

        // …otherwise, there is no `tail` or `head` item yet.
        item.detach();
        item.list = this;
        this.head = item;
        this.size++;

        return item;
    }

    /**
     * Creates an iterator from the list.
     *
     * @returns {ItemIterator<T>}
     */
    [Symbol.iterator]()
    {
        return new ItemIterator(this.head);
    }
}
