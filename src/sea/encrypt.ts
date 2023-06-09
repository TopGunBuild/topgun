import Buffer from 'topgun-buffer';
import WebCrypto from 'topgun-webcrypto';
import textEncoder from 'topgun-textencoder';
import { isObject, isString } from 'topgun-typed';
import { importAesKey } from './import-aes-key';
import { Pair } from './pair';
import { random } from '../utils/random';

const DEFAULT_OPTS: {
    readonly name?: string;
    readonly encode?: string;
    readonly raw?: boolean;
} = {
    encode: 'base64',
    name  : 'AES-GCM',
};

export async function encrypt(
    msg: string,
    keyOrPair: string|Pair,
): Promise<string>;
export async function encrypt(
    msg: string,
    keyOrPair: string|Pair,
    opt: {
        readonly name?: string;
        readonly encode?: string;
        readonly raw?: false;
    },
): Promise<string>;
export async function encrypt(
    msg: string,
    keyOrPair: string|Pair,
    opt: {
        readonly name?: string;
        readonly encode?: string;
        readonly raw: true;
    },
): Promise<{ct: string; iv: string; readonly s: string}>;
export async function encrypt(
    msg: string,
    keyOrPair: string|Pair,
    opt = DEFAULT_OPTS,
): Promise<string|{ct: string; iv: string; readonly s: string}>
{
    const rand = { s: random(9), iv: random(15) }; // consider making this 9 and 15 or 18 or 12 to reduce == padding.

    const key      =
              isObject(keyOrPair) && isString(keyOrPair.epriv)
                  ? keyOrPair.epriv
                  : isString(keyOrPair)
                      ? keyOrPair
                      : '';
    const ct       = await WebCrypto.subtle.encrypt(
        {
            iv  : new Uint8Array(rand.iv),
            name: opt.name || DEFAULT_OPTS.name || 'AES-GCM',
        },
        await importAesKey(key, rand.s),
        textEncoder.encode(msg),
    );
    const encoding = opt.encode || DEFAULT_OPTS.encode;
    const r        = {
        ct: Buffer.from(ct, 'binary').toString(encoding),
        iv: rand.iv.toString(encoding),
        s : rand.s.toString(encoding),
    };
    if (opt.raw)
    {
        return r;
    }
    return 'SEA' + JSON.stringify(r);
}
