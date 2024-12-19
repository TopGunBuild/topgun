import { mkenum } from "@topgunbuild/common"

/** Represents a keypair with raw byte arrays */
export type ByteKeypairInfo = {
    /** Public key as raw bytes */
    publicKey: Uint8Array
    /** Secret/private key as raw bytes */
    secretKey: Uint8Array
}

/** Represents a keypair with Base58 encoded strings */
export type Base58KeypairInfo = {
    /** Public key encoded in Base58 */
    publicKey: string
    /** Secret/private key encoded in Base58 */
    secretKey: string
}

/** 
 * Represents a cryptographically signed message with its verification data
 * Used for message authentication and integrity verification
 */
export type SignedMessageInfo = {
    /** The original message/payload to be verified */
    payload: any
    /** Cryptographic signature encoded in Base58 */
    signature: string
    /** Signer's public key encoded in Base58 for verification */
    publicKey: string
}

/** 
 * Represents an encrypted message with its nonce
 * Used for symmetric encryption operations
 */
export type CipherInfo = {
    /** Random nonce used for encryption */
    nonce: Uint8Array
    /** The encrypted message content */
    message: Uint8Array
}

/** Type for functions that encode byte arrays to strings */
export type Encoder = (b: Uint8Array) => string

/** Type alias for passwords that can be either strings or byte arrays */
export type Password = string | Uint8Array

/** 
 * Defines the available key types in the system
 * Can be extended by applications for custom key types
 */
export const KeyType = mkenum({
    TEAM: 'TEAM',
    ROLE: 'ROLE',
    USER: 'USER',
    DEVICE: 'DEVICE',
    SERVER: 'SERVER',
    EPHEMERAL: 'EPHEMERAL'
});
export type KeyType = (typeof KeyType)[keyof typeof KeyType];

/**
 * Defines the scope of a keyset, identifying its purpose and owner
 * Examples:
 * - User scope: {type: 'USER', name: 'alice'}
 * - Device scope: {type: 'DEVICE', name: 'laptop'}
 * - Role scope: {type: 'ROLE', name: 'MANAGER'}
 * - Ephemeral scope: {type: 'EPHEMERAL', name: 'EPHEMERAL'}
 */
export interface KeyScopeInfo {
    /** The type of the key scope (can be extended beyond KeyType) */
    type: string
    /** Identifier within the scope type */
    name: string
}

/**
 * Extends KeyScope with a generation number for key rotation purposes
 */
export interface KeyBaseInfo extends KeyScopeInfo {
    /** Incremental number indicating the key generation/version */
    generation: number
}

/**
 * Represents a key manifest containing public key information and metadata
 * Used to identify and verify keys without exposing private information
 */
export interface KeyManifest extends KeyBaseInfo {
    /** Public key encoded in Base58 format */
    publicKey: string
}

/**
 * Complete keyset containing both public and private keys
 * Used for all cryptographic operations (encryption, decryption, signing)
 */
export interface KeysetPrivateInfo extends KeyBaseInfo {
    /** Secret key for symmetric encryption */
    secretKey: string
    /** Keypair for asymmetric encryption operations */
    encryption: Base58KeypairInfo
    /** Keypair for digital signatures */
    signature: Base58KeypairInfo
}

/**
 * Public part of a keyset containing only public keys
 * Used for sharing and verification purposes
 */
export interface KeysetPublicInfo extends KeyBaseInfo {
    /** Public key for asymmetric encryption */
    encryption: string
    /** Public key for signature verification */
    signature: string
}

/**
 * Collection of keysets indexed by their public encryption keys
 * Provides a way to manage multiple keysets for different purposes
 */
export type Keyring = Record<string, KeysetPrivateInfo>;

/**
 * Collection of keysets indexed by their public encryption keys
 * Provides a way to manage multiple keysets for different purposes
 */
export type KeyMap = Record<string, Record<string, KeysetPrivateInfo[]>>;