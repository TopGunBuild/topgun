import WebCrypto from 'topgun-webcrypto';
import Buffer from 'topgun-buffer';

export const random = (len: number): any => Buffer.from(WebCrypto.getRandomValues(new Uint8Array(Buffer.alloc(len))));