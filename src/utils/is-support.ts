import { isObject, isString, isNumber, isBoolean } from 'topgun-typed';
import { TGValue } from '../types';

export function isSupportValue(value: unknown): value is TGValue
{
    return (
        isObject(value) ||
        isString(value) ||
        isBoolean(value) ||
        // we want +/- Infinity to be, but JSON does not support it, sad face.
        (isNumber(value) &&
            value !== Infinity &&
            value !== -Infinity &&
            !Number.isNaN(value)) ||
        value === null
    );
}
