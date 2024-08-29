import { type Base58Keypair } from './crypto';

export const KeyType = {
    GRAPH : 'GRAPH',
    USER  : 'USER',
    DEVICE: 'DEVICE',
    ROLE  : 'ROLE',
} as const;
export type KeyType = (typeof KeyType)[keyof typeof KeyType];

export interface KeyPair
{
    privateKey: Uint8Array;
    publicKey: Uint8Array;
}

/**
 * KeyScope represents the scope of a keyset. For example:
 * - a user: `{type: USER, name: 'alice'}`
 * - a device: `{type: DEVICE, name: 'laptop'}`
 * - a role: `{type: ROLE, name: 'MANAGER'}`
 * - a single-use keyset: `{type: EPHEMERAL, name: EPHEMERAL}`
 */
export type KeyScope = {
    /** The apps are not limited to KeyType, as they will have their own types. */
    type: string;
    name: string;
}

export type KeyMetadata = KeyScope&{
    generation: number;
}

/**
 * A Keyset includes the public encryption and signature keys from
 * a KeysetWithSecrets.
 */
export type Keyset = KeyMetadata&{
    /** Encryption publicKey */
    encryption: string;
    /** Signature publicKey */
    signature: string;
}

/**
 * A Keyset includes one secret key for symmetric encryption,
 * as well as two key pairs for asymmetric encryption and signatures.
 */
export type KeysetWithSecrets = KeyMetadata&{
    /** For symmetric encryption */
    secretKey: string;
    /** For symmetric encryption */
    encryption: Base58Keypair;
    signature: Base58Keypair;
}

/**
 * A keyring is a collection of key sets (including secrets) that are
 * organized by the public part of the asymmetric encryption key.
 */
export type Keyring = Record<string, KeysetWithSecrets>;
