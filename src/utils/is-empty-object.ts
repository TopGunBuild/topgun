import { isObject } from './is-object';

export function isNotEmptyObject(value: any): boolean 
{
    return isObject(value) && Object.keys(value).length > 0;
}

export function isEmptyObject(value: any): boolean 
{
    return isObject(value) && Object.keys(value).length === 0;
}
