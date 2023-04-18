import root from '../utils/window-or-global';
import WebCrypto from 'topgun-webcrypto';

const api: any = {
    crypto: WebCrypto,
    TextEncoder: root && root.TextEncoder,
    TextDecoder: root && root.TextDecoder,
};

api.random = (len: number) =>
    Buffer.from(crypto.getRandomValues(new Uint8Array(Buffer.alloc(len))));

export const random = api.random;
export const TextEncoder = api.TextEncoder;
export const TextDecoder = api.TextDecoder;
export const crypto = api.crypto;
