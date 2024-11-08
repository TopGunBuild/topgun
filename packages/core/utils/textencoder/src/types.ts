export interface ITextEncoder
{
    encode(input: string): Uint8Array;

    decode(input: ArrayBuffer): string;
}
