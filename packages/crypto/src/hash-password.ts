import { base58 } from '@scure/base';
import { argon2id } from '@noble/hashes/argon2';
import type { Password } from './types';
import { keyToBytes } from './utils';

/**
 * Uses the Argon2id algorithm to create a key from a low-entropy input, like a password.
 */
export const hashPassword = (password: Password): Uint8Array =>
{
    const passwordBytes = typeof password === 'string'
        ? keyToBytes(password, 'utf8')
        : password;
    const salt          = base58.decode('H5B4DLSXw5xwNYFdz1Wr6e');

    return argon2id(passwordBytes, salt, {
        t    : 3, // 3 iterations
        m    : 4096, // The value is set to 4 MiB, which is different from the recommended 64 MiB (65536). This change is made to speed things up, as 64 MiB takes more than 5 seconds on a cheap tablet.
        p    : 4, // 4 lanes
        dkLen: 32, // The tag size is 256 bits.
    });
};

