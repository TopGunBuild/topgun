import WebCrypto from '@topgunbuild/webcrypto';
import Buffer from '@topgunbuild/buffer';

export const random = (len: number): any => Buffer.from(WebCrypto.getRandomValues(new Uint8Array(Buffer.alloc(len))));