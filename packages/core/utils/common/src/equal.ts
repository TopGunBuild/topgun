import { isObject } from './typed';

export const equal = (a: unknown, b: unknown): boolean =>
{
    if (a === b)
    {
        return true;
    }

    if (isObject(a) && isObject(b))
    {
        for (const p in a)
        {
            if (a.hasOwnProperty(p) !== b.hasOwnProperty(p))
            {
                return false;
            }
            if (a[p] != b[p])
            {
                return false;
            }
        }

        for (const p in b)
        {
            if (typeof (a[p]) == 'undefined')
            {
                return false;
            }
        }

        return true;
    }

    if (Array.isArray(a) && Array.isArray(b))
    {
        let length: number, i: number;

        length = a.length;
        if (length != b.length)
        {
            return false;
        }
        for (i = length; i-- !== 0;)
        {
            if (!equal(a[i], b[i]))
            {
                return false;
            }
        }
        return true;
    }

    return a !== a && b !== b;
};
