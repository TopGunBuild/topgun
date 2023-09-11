export function createSoul(...args: any[]): string
{
    return args
        .reduce((accum: string[], item) => Array.isArray(item) ? [...accum, createSoul(...item)] : [...accum, `${item}`], [])
        .map(item => item.replace(/\s/g, ''))
        .join('/');
}
