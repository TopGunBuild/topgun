import { base58 } from '@scure/base';

/**
 * Encodes a Uint8Array or string into base58
 * @param value Uint8Array or string representing a borsh encoded object
 * @returns string base58 encoding of the value
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
 * Decodes a base58 string into a Uint8Array
 * @param value base58 encoded string
 * @returns Uint8Array representing the decoded value
 */
export function baseDecode(value: string): Uint8Array
{
    return new Uint8Array(base58.decode(value));
}
