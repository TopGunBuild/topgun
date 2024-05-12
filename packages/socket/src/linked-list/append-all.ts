import { Item } from './item';
import { LinkedList } from './list';

/**
 * Creates a new list from the items passed in.
 */
export function appendAll<T extends Item>(
    list: LinkedList<T>,
    items: Array<T | null | undefined> | undefined
): LinkedList<T>
{
    if (!items)
    {
        return list;
    }

    if (items[Symbol.iterator])
    {
        const iterator = items[Symbol.iterator]();
        /** @type {IteratorResult<T|null|undefined, null>} */
        let result;

        while ((result = iterator.next()) && !result.done)
        {
            list.append(result.value);
        }
    }
    else
    {
        let index = -1;

        while (++index < items.length)
        {
            const item = items[index];
            list.append(item);
        }
    }

    return list;
}
