import { LinkedList } from './list';

/**
 * Double linked list item.
 */
export class Item 
{
    next: Item | null;
    prev: Item | null;
    list: LinkedList<Item> | null;

    /**
     * Create a new linked list item.
     */
    constructor() 
    {
        this.next = null;
        this.prev = null;
        this.list = null;
    }

    /**
     * Add the given item **after** the operated on item in a list.
     *
     * Throws an error when the given item has no `detach`, `append`, or
     * `prepend` methods.
     * Returns `false` when the operated on item is not attached to a list,
     * otherwise the given item.
     */
    append(item: Item): Item | false 
    {
        const list = this.list;

        if (!item || !item.append || !item.prepend || !item.detach) 
        {
            throw new Error(
                'An argument without append, prepend, or detach methods was given to `Item#append`.'
            );
        }

        // If self is detached or appending ourselves, return false.
        if (!list || this === item) 
        {
            return false;
        }

        // Detach the appendee.
        item.detach();

        // If self has a next item…
        if (this.next) 
        {
            item.next = this.next;
            this.next.prev = item;
        }

        // Connect the appendee.
        item.prev = this;
        item.list = list;

        // Set the next item of self to the appendee.
        this.next = item;

        // If the the parent list has no last item or if self is the parent lists last
        // item, link the lists last item to the appendee.
        if (this === list.tail || !list.tail) 
        {
            list.tail = item;
        }

        list.size++;

        return item;
    }

    /**
     * Add the given item **before** the operated on item in a list.
     *
     * Throws an error when the given item has no `detach`, `append`, or `prepend`
     * methods.
     * Returns `false` when the operated on item is not attached to a list,
     * otherwise the given item.
     */
    prepend(item: Item): Item | false 
    {
        const list = this.list;

        if (!item || !item.append || !item.prepend || !item.detach) 
        {
            throw new Error(
                'An argument without append, prepend, or detach methods was given to `Item#prepend`.'
            );
        }

        // If self is detached or prepending ourselves, return false.
        if (!list || this === item) 
        {
            return false;
        }

        // Detach the prependee.
        item.detach();

        // If self has a previous item...
        if (this.prev) 
        {
            item.prev = this.prev;
            this.prev.next = item;
        }

        // Connect the prependee.
        item.next = this;
        item.list = list;

        // Set the previous item of self to the prependee.
        this.prev = item;

        // If self is the first item in the parent list, link the lists first item to
        // the prependee.
        if (this === list.head) 
        {
            list.head = item;
        }

        // If the the parent list has no last item, link the lists last item to self.
        if (!list.tail) 
        {
            list.tail = this;
        }

        list.size++;

        return item;
    }

    /**
     * Remove the operated on item from its parent list.
     *
     * Removes references to it on its parent `list`, and `prev` and `next`
     * items.
     * Relinks all references.
     * Returns the operated on item.
     * Even when it was already detached.
     */
    detach(): Item 
    {
        const list = this.list;

        if (!list) 
        {
            return this;
        }

        // If self is the last item in the parent list, link the lists last item to
        // the previous item.
        if (list.tail === this) 
        {
            list.tail = this.prev;
        }

        // If self is the first item in the parent list, link the lists first item to
        // the next item.
        if (list.head === this) 
        {
            list.head = this.next;
        }

        // If both the last and first items in the parent list are the same, remove
        // the link to the last item.
        if (list.tail === list.head) 
        {
            list.tail = null;
        }

        // If a previous item exists, link its next item to selfs next item.
        if (this.prev) 
        {
            this.prev.next = this.next;
        }

        // If a next item exists, link its previous item to selfs previous item.
        if (this.next) 
        {
            this.next.prev = this.prev;
        }

        // Remove links from self to both the next and previous items, and to the
        // parent list.
        this.prev = null;
        this.next = null;
        this.list = null;

        list.size--;

        return this;
    }
}
