import { base58 } from '@scure/base';

/**
 * Encode a Uint8Array or string into base58.
 */
export function baseEncode(value: Uint8Array|string): string
{
    if (typeof value === 'string')
    {
        const bytes = [];
        for (let c = 0; c < value.length; c++)
        {
            bytes.push(value.charCodeAt(c));
        }
        value = new Uint8Array(bytes);
    }
    return base58.encode(value);
}

/**
 * Decode a base58 string into a Uint8Array.
 */
export function baseDecode(value: string): Uint8Array
{
    return new Uint8Array(base58.decode(value));
}
