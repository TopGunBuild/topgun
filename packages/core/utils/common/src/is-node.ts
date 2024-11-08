export function isNode(): boolean
{
    return typeof process === 'object';
}
