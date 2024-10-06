import { base58 } from '@scure/base';
import { pbkdf2 } from '@noble/hashes/pbkdf2';
import { sha256 } from '@noble/hashes/sha2';
import type { Password } from './types';
import { keyToBytes } from './utils';
import { hashBytes } from './hash';

/**
 * Uses the pbkdf2 algorithm to derive a key from a low-entropy input, such as a password
 */
export const hashPassword = (password: Password): Uint8Array =>
{
    const passwordBytes = typeof password === 'string'
        ? keyToBytes(password, 'utf8')
        : password;
    const salt          = base58.decode('H5B4DLSXw5xwNYFdz1Wr6e');

    // It's long enough. Just hash it to expand it to 32 bytes
    if (passwordBytes.length >= 16)
    {
        return hashBytes(salt, passwordBytes);
    }

    return pbkdf2(sha256, passwordBytes, salt, { c: 32, dkLen: 32 });
};

