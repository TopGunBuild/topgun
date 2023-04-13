import { isObject } from './is-object';
import { isSupport } from './is-support';

export function checkType(d: any, tmp?: any): string 
{
    return (d && (tmp = d.constructor) && tmp.name) || typeof d;
}

export function set(list: Array<string>, value: any): { [key: string]: any } 
{
    return list.reverse().reduce((a, c) => ({ [c]: a }), value);
}

export function dataWalking(
    obj: any,
    pathArr: string[] = [],
    target = {},
): { [key: string]: any } 
{
    if (!isSupport(obj)) 
    {
        throw Error(
            'Invalid data: ' + checkType(obj) + ' at ' + pathArr.join('.'),
        );
    }
    else if (!isObject(obj)) 
    {
        obj = set(pathArr, obj);
    }

    const path = pathArr.join('/');
    if (pathArr.length > 0 && !isObject(target[path])) 
    {
        target[path] = {};
    }

    for (const k in obj) 
    {
        if (!obj.hasOwnProperty(k)) 
        {
            continue;
        }

        const value = obj[k];
        const pathArrFull = [...pathArr, k];
        const pathFull = pathArrFull.join('/');

        if (!isSupport(value)) 
        {
            console.log(
                'Invalid data: ' +
                    checkType(value) +
                    ' at ' +
                    pathArrFull.join('.'),
            );
            continue;
        }

        if (isObject(value)) 
        {
            target[path][k] = { '#': pathFull };
            dataWalking(value, pathArrFull, target);
        }
        else 
        {
            target[path][k] = value;
        }
    }
    return target;
}
