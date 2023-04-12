export function isFunction(value): value is (...params: any[]) => any 
{
    return typeof value === 'function';
}
