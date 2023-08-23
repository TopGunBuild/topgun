import Buffer from '@topgunbuild/buffer';
import WebCrypto from '@topgunbuild/webcrypto';
import { isObject, isString } from '@topgunbuild/typed';
import { ecdsa, jwk, parse } from './settings';
import { sha256 } from './sha256';
import { Pair } from './pair';
import { TGEncryptData } from '../types';

const DEFAULT_OPTS: {
    readonly encode?: string;
    readonly raw?: boolean;
    readonly check?: {
        readonly m: any;
        readonly s: string;
    };
} = {
    encode: 'base64',
};

function importKey(pub: string): Promise<any>
{
    const token   = jwk(pub);
    const promise = WebCrypto.subtle.importKey('jwk', token, ecdsa.pair, false, [
        'verify',
    ]);
    return promise;
}

export async function verifyHashSignature(
    hash: string,
    signature: string,
    pub: string,
    opt = DEFAULT_OPTS,
): Promise<boolean>
{
    const encoding = opt.encode || DEFAULT_OPTS.encode;
    const key      = await importKey(pub);
    const buf      = Buffer.from(signature, encoding);
    const sig      = new Uint8Array(buf);

    if (
        await WebCrypto.subtle.verify(
            ecdsa.sign,
            key,
            sig,
            new Uint8Array(Buffer.from(hash, 'hex')),
        )
    )
    {
        return true;
    }

    return false;
}

export async function verifySignature(
    text: string,
    signature: string,
    pub: string,
    opt = DEFAULT_OPTS,
): Promise<boolean>
{
    const hash = await sha256(isString(text) ? text : JSON.stringify(text));
    return verifyHashSignature(hash.toString('hex'), signature, pub, opt);
}

export async function verify(
    data: string|{readonly m: string; readonly s: string},
    pubOrPair: string|Pair,
    opt = DEFAULT_OPTS,
): Promise<false|TGEncryptData>
{
    try
    {
        const pub =
                  isObject(pubOrPair) && isString(pubOrPair.pub)
                      ? pubOrPair.pub
                      : isString(pubOrPair)
                          ? pubOrPair
                          : '';

        const json = parse(data);
        if (await verifySignature(json.m, json.s, pub, opt))
        {
            return {
                ct: json.ct,
                iv: json.iv,
                s : json.s,
                e : json.m?.e,
                w : json.m?.w,
                c : json.m?.c,
            };
        }
        return false;
    }
    catch (e)
    {
        return false;
    }
}
