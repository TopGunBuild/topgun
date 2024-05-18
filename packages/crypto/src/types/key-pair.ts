import { Signature } from './signature';
import { PublicKey } from './public-key';

export abstract class KeyPair
{
    abstract sign(message: Uint8Array): Signature;

    abstract verify(message: Uint8Array, signature: Uint8Array): boolean;

    abstract toString(): string;

    abstract getPublicKey(): PublicKey;
}
