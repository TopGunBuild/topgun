import WebCrypto from 'topgun-webcrypto';
import { Pair } from './pair';

const keysToEcdhJwk = (
    pub: string,
    d?: string,
): ['jwk', JsonWebKey, EcKeyImportParams] =>
{
    const [x, y] = pub.split('.'); // new
    const jwk    = d ? { d } : {};
    return [
        // Use with spread returned value...
        'jwk',
        Object.assign(jwk, { x: x, y: y, kty: 'EC', crv: 'P-256', ext: true }), // ??? refactor
        { name: 'ECDH', namedCurve: 'P-256' },
    ];
};

export async function secret(
    key: string,
    pair: Pair,
    cb?: (value?: string) => void,
): Promise<string|undefined>
{
    try
    {
        if (!pair || !pair.epriv || !pair.epub)
        {
            console.log('No secret mix.');
            return;
        }

        const pub                          = key;
        const epub                         = pair.epub;
        const epriv                        = pair.epriv;
        const [format, keyData, algorithm] = keysToEcdhJwk(pub);
        const props                        = Object.assign(
            {
                public: await WebCrypto.subtle.importKey(
                    format,
                    keyData,
                    algorithm,
                    true,
                    [],
                ),
            },
            {
                name      : 'ECDH',
                namedCurve: 'P-256',
            },
        );
        const privKeyData                  = keysToEcdhJwk(epub, epriv);
        const derived                      = await WebCrypto.subtle
            .importKey(...privKeyData, false, ['deriveBits'])
            .then(async (privKey) =>
            {
                // privateKey scope doesn't leak out from here!
                const derivedBits = await WebCrypto.subtle.deriveBits(
                    props,
                    privKey,
                    256,
                );
                const rawBits     = new Uint8Array(derivedBits);
                const derivedKey  = await WebCrypto.subtle.importKey(
                    'raw',
                    rawBits,
                    {
                        name  : 'AES-GCM',
                        length: 256,
                    },
                    true,
                    ['encrypt', 'decrypt'],
                );

                return WebCrypto.subtle
                    .exportKey('jwk', derivedKey)
                    .then(({ k }) => k);
            });

        const r = derived;
        if (cb)
        {
            try
            {
                cb(r);
            }
            catch (e)
            {
                console.log(e);
            }
        }
        return r;
    }
    catch (e)
    {
        console.error(e);
        if (cb)
        {
            cb();
        }
    }
}
