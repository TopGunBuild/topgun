import { ITextEncoder } from './types';

export const textEncoder: ITextEncoder = {
    encode: (input: string): Uint8Array => new TextEncoder().encode(input),
    decode: (input: ArrayBuffer): string => new TextDecoder().decode(input),
};
