import Buffer from 'topgun-buffer';
import WebCrypto from 'topgun-webcrypto';
import textEncoder from 'topgun-textencoder';
import { isString, isObject } from 'topgun-typed';
import { importAesKey } from './import-aes-key';
import { parse } from './settings';
import { Pair } from './pair';

const DEFAULT_OPTS: {
    readonly name?: string;
    readonly encode?: string;
    readonly fallback?: string;
} = {
    encode: 'base64',
    name  : 'AES-GCM',
};

export async function decrypt<T>(
    data: string|{ct: string; iv: string; readonly s: string},
    keyOrPair: string|Pair,
    opt = DEFAULT_OPTS,
): Promise<T|undefined>
{
    const json: any = parse(data);
    const encoding  = opt.encode || DEFAULT_OPTS.encode;
    const key       =
              isObject(keyOrPair) && isString(keyOrPair.epriv)
                  ? keyOrPair.epriv
                  : isString(keyOrPair)
                      ? keyOrPair
                      : '';

    try
    {
        const aeskey    = await importAesKey(key, Buffer.from(json.s, encoding));
        const encrypted = new Uint8Array(Buffer.from(json.ct, encoding));
        const iv        = new Uint8Array(Buffer.from(json.iv, encoding));
        const ct        = await WebCrypto.subtle.decrypt(
            {
                iv,
                name     : opt.name || DEFAULT_OPTS.name || 'AES-GCM',
                tagLength: 128,
            },
            aeskey,
            encrypted,
        );
        return parse(textEncoder.decode(ct));
    }
    catch (e: any)
    {
        // console.warn('decrypt error', e, e.stack || e);

        if (!opt.fallback || encoding === opt.fallback)
        {
            return;
            // throw new Error('Could not decrypt');
        }
        return decrypt(data, key, { ...opt, encode: opt.fallback });
    }
}
