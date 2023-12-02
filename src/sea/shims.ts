import webCrypto from '@topgunbuild/webcrypto';
import textEncoder from '@topgunbuild/textencoder';
import buffer from '@topgunbuild/buffer';

export const crypto      = webCrypto['default'] || webCrypto;
export const TextEncoder = textEncoder['default'] || textEncoder;
export const Buffer      = buffer['default'] || buffer;
export const random = (len: number): any => Buffer.from(crypto.getRandomValues(new Uint8Array(Buffer.alloc(len))));


