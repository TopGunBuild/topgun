import { x25519 } from '@noble/curves/ed25519';
import { randomBytes } from '@noble/hashes/utils';
import { deserialize, serialize } from '@dao-xyz/borsh';
import { base58 } from '@scure/base';
import { secretbox } from '@noble/ciphers/salsa';
import type { Base58Keypair, KeyPair } from '@topgunbuild/types';
import { hashPassword } from './hash-password';
import { keypairToBase58, keyToBytes } from './utils';
import { Cipher } from './cipher';
import { NONCE_LENGTH } from './const';

/**
 * @returns A pair of keys, a public key and a secret key, encoded as base58
 * strings, is used for asymmetric encryption and decryption.
 */
const keyPair = (seed?: string): Base58Keypair =>
{
    const privateKey = seed
        ? hashPassword(seed)
        : x25519.utils.randomPrivateKey();
    const publicKey  = x25519.getPublicKey(privateKey);

    const keypair: KeyPair = { privateKey, publicKey };
    return keypairToBase58(keypair);
};

const encryptBytes = ({ payload, recipientPublicKey, senderSecretKey }: {
    payload: Uint8Array
    recipientPublicKey: string
    senderSecretKey?: string
}): Uint8Array =>
{
    const nonce = randomBytes(NONCE_LENGTH);

    let senderPublicKey: string|undefined;
    if (senderSecretKey === undefined)
    {
        // For secure communications, use ephemeral sender keys
        const senderKeys = asymmetric.keyPair();
        senderSecretKey  = senderKeys.secretKey;
        senderPublicKey  = senderKeys.publicKey;
    }
    else
    {
        // Use the provided sender keys and do not include public key in the metadata
        senderPublicKey = undefined;
    }

    const sharedSecretKey = x25519.getSharedSecret(
        keyToBytes(senderSecretKey),
        keyToBytes(recipientPublicKey)
    );

    // Encrypt message
    const message = secretbox(sharedSecretKey, nonce).seal(payload);
    const cipher  = new Cipher({ nonce, message, senderPublicKey });
    return serialize(cipher);
};

const decryptBytes = ({ cipher, recipientSecretKey, senderPublicKey }: {
    cipher: Uint8Array
    senderPublicKey?: string
    recipientSecretKey: string
}): Uint8Array =>
{
    const unpackedCipher     = deserialize(cipher, Cipher);
    const { nonce, message } = unpackedCipher;

    // If the sender's public key is not included, it means that an ephemeral public key is included in the metadata
    senderPublicKey = senderPublicKey ?? unpackedCipher.senderPublicKey;

    const sharedSecretKey = x25519.getSharedSecret(
        keyToBytes(recipientSecretKey),
        keyToBytes(senderPublicKey!)
    );

    return secretbox(sharedSecretKey, nonce).open(message);
};

/**
 * Asymmetrically encrypts a byte array. If no sender secret key is provided,
 * an ephemeral keypair will be generated, and the public key will be included
 * as metadata.
 */
const encrypt = ({
                     payload,
                     recipientPublicKey,
                     senderSecretKey,
                 }: {
    payload: Uint8Array
    recipientPublicKey: string
    senderSecretKey?: string
}): string =>
{
    const cipherBytes = encryptBytes({ payload, recipientPublicKey, senderSecretKey });
    return base58.encode(cipherBytes);
};

/**
 * Asymmetrically decrypts a message. If the sender's public key is not
 * provided, an ephemeral public key is assumed to be included in the cipher metadata.
 */
const decrypt = ({ cipher, recipientSecretKey, senderPublicKey }: {
    cipher: string
    senderPublicKey?: string
    recipientSecretKey: string
}): Uint8Array =>
{
    const cipherBytes = keyToBytes(cipher);
    return decryptBytes({ cipher: cipherBytes, recipientSecretKey, senderPublicKey });
};

export const asymmetric = {
    keyPair,
    encryptBytes,
    decryptBytes,
    encrypt,
    decrypt,
};
