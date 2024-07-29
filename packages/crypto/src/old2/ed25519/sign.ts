import { ed25519 } from '@noble/curves/ed25519';
import { Ed25519Keypair } from './keypair';
import { PreHash, createHash } from '../hash';
import { Signature } from '../signature';

export const sign = async (
    data: Uint8Array,
    keypair: Ed25519Keypair,
    preHash: PreHash
): Promise<Signature> =>
{
    const hashedData = createHash(data, preHash);

    return new Signature({
        preHash  : preHash,
        publicKey: keypair.publicKey,
        signature: ed25519.sign(hashedData, keypair.extendedSecretKey)
    });
};

