import Buffer from '@topgunbuild/buffer';
import WebCrypto from '@topgunbuild/webcrypto';
import TextEncoder from '@topgunbuild/textencoder';
import { isString } from '@topgunbuild/typed';

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
