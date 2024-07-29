import { PublicKey } from './keys';

export abstract class Keypair
{
    abstract get publicKey(): PublicKey;

    equals(other: Keypair): boolean
    {
        throw new Error('Not implemented');
    }
}
