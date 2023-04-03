import { Pair } from './pair';
import { jwk } from './settings';
import { crypto } from './shims';

const DEFAULT_OPTS: {} = {};

const keysToEcdhJwk = (
    pub: string,
    d?: string
) =>
{
    const [x, y] = pub.split('.'); // new
    const jwk    = d ? { d } : {};
    return [  // Use with spread returned value...
        'jwk',
        Object.assign(
            jwk,
            { x: x, y: y, kty: 'EC', crv: 'P-256', ext: true }
        ), // ??? refactor
        { name: 'ECDH', namedCurve: 'P-256' }
    ];
};

/**
 * Derive shared secret from other's pub and my epub/epriv
 *
 * @param {string} key
 * @param {Pair} pair
 * @param {(value?: string) => void} cb
 * @param {{}} opt
 * @returns {Promise<string>}
 */
export async function secret(
    key: string,
    pair: Pair,
    cb?: (value?: string) => void,
    opt = DEFAULT_OPTS
): Promise<string>
{
    try
    {
        if (!pair || !pair.epriv || !pair.epub)
        {
            console.log('No secret mix.');
            return null;
        }

        const pub         = key;
        const epub        = pair.epub;
        const epriv       = pair.epriv;
        const pubKeyData  = keysToEcdhJwk(pub);
        const props       = Object.assign(
            {
                public: await crypto.subtle.importKey(...pubKeyData, true, [])
            },
            {
                name      : 'ECDH',
                namedCurve: 'P-256'
            }
        );
        const privKeyData = keysToEcdhJwk(epub, epriv);
        const derived     = await crypto.subtle
            .importKey(...privKeyData, false, ['deriveBits'])
            .then(async (privKey) =>
            {
                // privateKey scope doesn't leak out from here!
                const derivedBits = await crypto.subtle.deriveBits(props, privKey, 256);
                const rawBits     = new Uint8Array(derivedBits);
                const derivedKey  = await crypto.subtle.importKey('raw', rawBits, {
                    name  : 'AES-GCM',
                    length: 256
                }, true, ['encrypt', 'decrypt']);

                return crypto.subtle
                    .exportKey('jwk', derivedKey)
                    .then(({ k }) => k);
            });

        const r = derived;
        if (cb)
        {
            try
            {
                cb(r)
            }
            catch (e)
            {
                console.log(e)
            }
        }
        return r;
    }
    catch (e)
    {
        console.log(e);
        if (cb)
        {
            cb()
        }
        return null;
    }
}
