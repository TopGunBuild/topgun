import { randomBytes } from '@noble/hashes/utils';
import { secretbox } from '@noble/ciphers/salsa';
import { base58 } from '@scure/base';
import { deserialize, serialize } from '@dao-xyz/borsh';
import { stretch } from './stretch';
import type { Password, Payload } from './types';
import { Cipher } from './cipher';
import { keyToBytes } from './utils';

const NONCE_LENGTH = 24;

/**
 * Encrypts a byte array in a symmetrical manner.
 */
const encryptBytes = (payload: Uint8Array, password: Password): Uint8Array =>
{
    const key     = stretch(password);
    const nonce   = randomBytes(NONCE_LENGTH);
    const message = secretbox(key, nonce).seal(payload);
    const cipher  = new Cipher({ nonce, message });
    return serialize(cipher);
};

/**
 * Decodes a message that was encoded using `symmetric.encryptBytes`.
 * This function returns the original array of bytes.
 */
const decryptBytes = (cipher: Uint8Array, password: Password): Uint8Array =>
{
    const key                = stretch(password);
    const { nonce, message } = deserialize(cipher, Cipher);
    return secretbox(key, nonce).open(message);
};

/**
 * Encrypt data using symmetric encryption and convert the encrypted data into a base58 string.
 */
const encrypt = (payload: Uint8Array, password: Password): string =>
{
    const cipherBytes = encryptBytes(payload, password);
    return base58.encode(cipherBytes);
};

/**
 * Symmetrically decrypts a message encrypted by `symmetric.encrypt`.
 */
const decrypt = (cipher: string, password: Password): Payload =>
{
    const cipherBytes = keyToBytes(cipher);
    return decryptBytes(cipherBytes, password);
};

export const symmetric = { encryptBytes, decryptBytes, encrypt, decrypt };
