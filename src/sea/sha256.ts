import Buffer from 'topgun-buffer';
import { crypto, TextEncoder } from './shims';
import { isString } from '../utils/is-string';

export async function sha256(
    input: string | object,
    name = 'SHA-256',
): Promise<Buffer> 
{
    const inp = isString(input) ? input : JSON.stringify(input);
    const encoded = new TextEncoder().encode(inp);
    const hash = await crypto.subtle.digest({ name }, encoded);
    return Buffer.from(hash);
}