import root from '../utils/window-or-global';
import './base64';
import SafeBuffer from './safe-buffer';
import { Crypto } from '@peculiar/webcrypto';

export const crypto = new Crypto();
export const Buffer = SafeBuffer;

const api: any = {
    Buffer,
    crypto,
    TextEncoder: root && root.TextEncoder,
    TextDecoder: root && root.TextDecoder,
};

api.random     = (len: number) =>
    api.Buffer.from(
        crypto.getRandomValues(new Uint8Array(api.Buffer.alloc(len)))
    );

// if (!api.TextEncoder)
// {
//     import('text-encoding').then(({ TextEncoder, TextDecoder }) =>
//     {
//         api.TextEncoder                    = TextEncoder;
//         api.TextDecoder                    = TextDecoder;
//     });
// }

export const random      = api.random;
export const TextEncoder = api.TextEncoder;
export const TextDecoder = api.TextDecoder;


