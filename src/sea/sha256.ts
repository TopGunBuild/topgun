import Buffer from 'topgun-buffer';
import WebCrypto from 'topgun-webcrypto';
import TextEncoder from 'topgun-textencoder';
import { isString } from 'topgun-typed';

export async function sha256(
    input: string|object,
    name = 'SHA-256',
): Promise<any>
{
    const inp     = isString(input) ? input : JSON.stringify(input);
    const encoded = TextEncoder.encode(inp);
    const hash    = await WebCrypto.subtle.digest({ name }, encoded);
    return Buffer.from(hash);
}
