import WebCrypto from 'topgun-webcrypto';
import { ecdh, ecdsa } from './settings';

export interface PairBase
{
    /** private key */
    priv: string;
    /** public key */
    pub: string;
}

export interface Pair extends PairBase
{
    /** private key for encryption */
    epriv: string;
    /** public key for encryption */
    epub: string;
}

export async function pair(): Promise<Pair>
{
    const signKeys = await WebCrypto.subtle.generateKey(ecdsa.pair, true, [
        'sign',
        'verify',
    ]);
    const signPub  = await WebCrypto.subtle.exportKey('jwk', signKeys.publicKey);
    const sa       = {
        priv: (await WebCrypto.subtle.exportKey('jwk', signKeys.privateKey)).d,
        pub : `${signPub.x}.${signPub.y}`,
    };

    const cryptKeys = await WebCrypto.subtle.generateKey(ecdh, true, [
        'deriveKey',
    ]);
    const cryptPub  = await WebCrypto.subtle.exportKey('jwk', cryptKeys.publicKey);
    const dh        = {
        epriv: (await WebCrypto.subtle.exportKey('jwk', cryptKeys.privateKey)).d,
        epub : `${cryptPub.x}.${cryptPub.y}`,
    };

    return {
        epriv: dh.epriv || '',
        epub : dh.epub,
        priv : sa.priv || '',
        pub  : sa.pub,
    };
}
