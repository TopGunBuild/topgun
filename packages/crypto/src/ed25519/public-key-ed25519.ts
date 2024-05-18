import { baseEncode } from '@topgunbuild/utils';
import { ed25519 } from '@noble/curves/ed25519';
import { field, fixedArray } from '@dao-xyz/borsh';
import { KeySize } from '../constants';
import { decodedPublicKeyFromString } from '../utils';
import { PublicKey } from '../types/public-key';

export class PublicKeyEd25519 implements PublicKey
{
    @field({ type: fixedArray('u8', KeySize.ED25519) })
    readonly data: Uint8Array;

    /**
     * Creates a PublicKey instance from a string or an existing PublicKey instance.
     * @param value The string or PublicKey instance to create a PublicKey from.
     * @returns {PublicKeyEd25519} The PublicKey instance.
     */
    static from(value: string|PublicKeyEd25519): PublicKeyEd25519
    {
        if (typeof value === 'string')
        {
            return PublicKeyEd25519.fromString(value);
        }
        return value;
    }

    /**
     * Creates a PublicKey instance from an encoded key string.
     * @param encodedKey The encoded key string.
     * @returns {PublicKeyEd25519} The PublicKey instance created from the encoded key string.
     */
    static fromString(encodedKey: string): PublicKeyEd25519
    {
        const decodedPublicKey = decodedPublicKeyFromString(encodedKey, KeySize.ED25519);
        return new PublicKeyEd25519(decodedPublicKey);
    }

    /**
     * Constructs an instance of the `PublicKeyEd25519` class.
     * @param {Uint8Array} data
     */
    constructor(data: Uint8Array)
    {
        this.data = data;
    }

    /**
     * Returns a string representation of the public key.
     * @returns {string} The string representation of the public key.
     */
    toString(): string
    {
        return `ed25519:${baseEncode(this.data)}`;
    }

    /**
     * Verifies a message signature using the public key.
     * @param message The message to be verified.
     * @param signature The signature to be verified.
     * @returns {boolean} `true` if the signature is valid, otherwise `false`.
     */
    verify(message: Uint8Array, signature: Uint8Array): boolean
    {
        return ed25519.verify(signature, message, this.data);
    }
}
