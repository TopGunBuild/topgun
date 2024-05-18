import { baseDecode, baseEncode, randomBytes } from '@topgunbuild/utils';
import { ed25519 } from '@noble/curves/ed25519';
import { field } from '@dao-xyz/borsh';
import { concatBytes } from '@noble/curves/abstract/utils';
import { KeyPair, Signature } from '../types';
import { PublicKeyEd25519 } from './public-key-ed25519';
import { KeySize } from '../constants';

/**
 * This class provides key pair functionality for Ed25519 curve:
 * generating key pairs, encoding key pairs, signing and verifying.
 */
export class KeyPairEd25519 extends KeyPair
{
    @field({ type: PublicKeyEd25519 })
    readonly publicKey: PublicKeyEd25519;

    @field({ type: 'string' })
    readonly secretKey: string;

    readonly extendedSecretKey: string;

    /**
     * Construct an instance of key pair given a secret key.
     * It's generally assumed that these are encoded in base58.
     * @param extendedSecretKey
     */
    constructor(extendedSecretKey: string)
    {
        super();
        const decoded          = baseDecode(extendedSecretKey);
        const secretKey        = new Uint8Array(decoded.slice(0, KeySize.ED25519));
        const publicKey        = ed25519.getPublicKey(new Uint8Array(secretKey));
        this.publicKey         = new PublicKeyEd25519(publicKey);
        this.secretKey         = baseEncode(secretKey);
        this.extendedSecretKey = extendedSecretKey;
    }

    /**
     * Generate a new random keypair.
     * @example
     * const keyRandom = KeyPair.fromRandom();
     * keyRandom.publicKey
     * // returns [PUBLIC_KEY]
     *
     * keyRandom.secretKey
     * // returns [SECRET_KEY]
     */
    static create(): KeyPairEd25519
    {
        const secretKey         = randomBytes(KeySize.ED25519);
        const publicKey         = ed25519.getPublicKey(new Uint8Array(secretKey));
        const extendedSecretKey = concatBytes(secretKey, publicKey);
        return new KeyPairEd25519(baseEncode(extendedSecretKey));
    }

    /**
     * Creates a key pair from an encoded key string.
     * @param encodedKey The encoded key string.
     * @returns {KeyPairEd25519} The key pair created from the encoded key string.
     */
    static fromString(encodedKey: string): KeyPairEd25519
    {
        const parts = encodedKey.split(':');
        if (parts.length === 1)
        {
            return new KeyPairEd25519(parts[0]);
        }
        else if (parts.length === 2)
        {
            switch (parts[0].toUpperCase())
            {
                case 'ED25519':
                    return new KeyPairEd25519(parts[1]);
                default:
                    throw new Error(`Unknown curve: ${parts[0]}`);
            }
        }
        else
        {
            throw new Error('Invalid encoded key format, must be <curve>:<encoded key>');
        }
    }

    /**
     * Signs a message using the key pair's secret key.
     * @param message The message to be signed.
     * @returns {Signature} The signature object containing the signature and the public key.
     */
    sign(message: Uint8Array): Signature
    {
        const signature = ed25519.sign(message, baseDecode(this.secretKey));
        return { signature, publicKey: this.publicKey };
    }

    /**
     * Verifies the signature of a message using the key pair's public key.
     * @param message The message to be verified.
     * @param signature The signature to be verified.
     * @returns {boolean} `true` if the signature is valid, otherwise `false`.
     */
    verify(message: Uint8Array, signature: Uint8Array): boolean
    {
        return this.publicKey.verify(message, signature);
    }

    /**
     * Returns a string representation of the key pair in the format 'ed25519:[extendedSecretKey]'.
     * @returns {string} The string representation of the key pair.
     */
    toString(): string
    {
        return `ed25519:${this.extendedSecretKey}`;
    }

    /**
     * Retrieves the public key associated with the key pair.
     * @returns {PublicKey} The public key.
     */
    getPublicKey(): PublicKeyEd25519
    {
        return this.publicKey;
    }
}

