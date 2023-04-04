import { isObject } from './is-object';
import { TGValue } from '../types';
import { isString } from './is-string';
import { isNumber } from './is-number';
import { isBoolean } from './is-boolean';

export function isSupport(value: any): value is TGValue
{
    return isObject(value)
        || isString(value)
        || isBoolean(value)
        // we want +/- Infinity to be, but JSON does not support it, sad face.
        || (isNumber(value) && value !== Infinity && value !== -Infinity && !Number.isNaN(value))
        || value === null;
}
