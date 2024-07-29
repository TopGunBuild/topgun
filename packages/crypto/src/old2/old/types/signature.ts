import { PublicKey } from './public-key';

export interface Signature
{
    signature: Uint8Array;
    publicKey: PublicKey;
}
