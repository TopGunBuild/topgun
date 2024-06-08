/**
 * Check if value is a string.
 */
export function isString(value: unknown): value is string
{
    return typeof value === 'string';
}

/**
 * Check if value is a valid number.
 */
export function isNumber(value: unknown): value is number
{
    return typeof value === 'number' && Number.isFinite(value);
}

/**
 * Check if value is a boolean.
 */
export function isBoolean(value: unknown): value is boolean
{
    return typeof value === 'boolean';
}

/**
 * Check if value is undefined.
 */
export function isUndefined(value: unknown): value is undefined
{
    return typeof value === 'undefined';
}

/**
 * Check if value is not undefined.
 */
export function isDefined(value: unknown): boolean
{
    return !isUndefined(value);
}

/**
 * Check if value is a valid Date.
 */
export function isDate(value: unknown): value is Date
{
    return value instanceof Date && Number.isFinite(value.getTime());
}

/**
 * Check if value is an array.
 */
export function isArray(value: unknown): value is any[]
{
    return Array.isArray(value);
}

/**
 * Check if value is null.
 */
export function isNull(value: unknown): value is null
{
    return value === null;
}

export type Obj = {[key: string]: any};

/**
 * Check if value is an object.
 */
export function isObject(value: unknown): value is Obj
{
    return typeof value === 'object' && !isNull(value) && !isArray(value);
}

/**
 * Check if value is an object and it not empty.
 */
export function isNotEmptyObject(value: unknown): boolean
{
    return isObject(value) && Object.keys(value).length > 0;
}

/**
 * Check if value is an object and it is empty.
 */
export function isEmptyObject(value: unknown): boolean
{
    return isObject(value) && Object.keys(value).length === 0;
}

/**
 * Check if value is a function.
 */
export function isFunction(value: unknown): value is (...params: any[]) => any
{
    return typeof value === 'function';
}
