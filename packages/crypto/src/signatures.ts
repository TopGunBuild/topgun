import { base58 } from '@scure/base';
import { ed25519 } from '@noble/curves/ed25519';
import { hashPassword } from './hash-password';
import { type Base58Keypair, type KeyPair, type SignedMessage } from '@topgunbuild/types';
import { keypairToBase58, keyToBytes } from './utils';

/**
 * A key pair comprises a public key and a secret key, encoded as base58 strings,
 * for signing and verifying messages.
 */
const keyPair = (seed?: string): Base58Keypair =>
{
    const privateKey = seed
        ? hashPassword(seed)
        : ed25519.utils.randomPrivateKey();
    const publicKey  = ed25519.getPublicKey(privateKey);

    const keypair: KeyPair = { privateKey, publicKey };
    return keypairToBase58(keypair);
};

/**
 * @returns A signature that is encoded as a base58 string.
 */
const sign = (payload: Uint8Array, secretKey: string): string =>
{
    const secretKeyBytes = keyToBytes(secretKey);
    const signatureBytes = ed25519.sign(payload, secretKeyBytes);
    return base58.encode(signatureBytes);
};

/**
 * @returns true if verification is successful, and false if it is not.
 */
const verify = ({ payload, signature, publicKey }: SignedMessage): boolean =>
{
    const signatureBytes = keyToBytes(signature);
    const publicKeyBytes = keyToBytes(publicKey);
    return ed25519.verify(signatureBytes, payload, publicKeyBytes);
};

export const signatures = { keyPair, sign, verify };
