import { isObject } from './typed';

export const equalObjects = (obj1: unknown, obj2: unknown) =>
{
    if (isObject(obj1) && isObject(obj2))
    {
        for (const p in obj1)
        {
            if (obj1.hasOwnProperty(p) !== obj2.hasOwnProperty(p))
            {
                return false;
            }
            if (obj1[p] != obj2[p])
            {
                return false;
            }
        }

        for (const p in obj2)
        {
            if (typeof (obj1[p]) == 'undefined')
            {
                return false;
            }
        }

        return true;
    }

    return obj1 === obj2;
};
