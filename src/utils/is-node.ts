import { isObject, isString } from 'topgun-typed';

export function isNode(data: unknown): boolean
{
    return isObject(data) &&
        isString(data && data._ && data._['#']) &&
        isObject(data && data._ && data._['>']);
}
