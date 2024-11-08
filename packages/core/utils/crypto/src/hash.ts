import { base58 } from '@scure/base';
import { blake2s } from '@noble/hashes/blake2s';
import { keyToBytes } from './utils';

/** Computes a fixed-length fingerprint for an arbitrary long message. */
export const hash = (seed: string, payload: Uint8Array): string =>
{
    return base58.encode(hashBytes(seed, payload));
};

export const hashBytes = (seed: string|Uint8Array, payload: Uint8Array): Uint8Array =>
{
    const seedBytes = typeof seed === 'string'
        ? keyToBytes(seed, 'utf8')
        : seed;
    return blake2s(payload, { dkLen: 32, key: seedBytes });
};
