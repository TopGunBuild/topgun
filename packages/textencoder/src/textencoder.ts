import { ITextEncoder } from './types';

export const textEncoder: ITextEncoder = {
    encode: (input: string) => new Uint8Array(Buffer.from(input, 'utf8')),
    decode: (input: ArrayBuffer) => Buffer.from(input).toString('utf8'),
};
