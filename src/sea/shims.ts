import root from '../utils/window-or-global';
import './base64';
import SafeBuffer from './safe-buffer';
import WebCrypto from 'topgun-webcrypto';

const api: any = {
    Buffer: SafeBuffer,
    crypto: WebCrypto,
    TextEncoder: root && root.TextEncoder,
    TextDecoder: root && root.TextDecoder,
};

api.random = (len: number) =>
    api.Buffer.from(
        api.crypto.getRandomValues(new Uint8Array(api.Buffer.alloc(len))),
    );

export const random = api.random;
export const TextEncoder = api.TextEncoder;
export const TextDecoder = api.TextDecoder;
