import Buffer from 'topgun-buffer';
import { crypto, TextEncoder } from './shims';
import { isString } from 'topgun-typed';

export async function sha256(
    input: string | object,
    name = 'SHA-256',
): Promise<any> 
{
    const inp = isString(input) ? input : JSON.stringify(input);
    const encoded = new TextEncoder().encode(inp);
    const hash = await crypto.subtle.digest({ name }, encoded);
    return Buffer.from(hash);
}
