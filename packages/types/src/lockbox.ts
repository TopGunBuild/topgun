import { Base58, KeyMetadata } from "./keyset"
import { Identifiable } from "./utils"

/**
 * Represents a key manifest containing public key information and metadata
 * Used to identify and verify keys without exposing private information
 */
export interface KeyManifest extends KeyMetadata {
    /** Public key encoded in Base58 format */
    publicKey: Base58
}

/**
 * Represents a secure container (lockbox) for transmitting encrypted keys
 * Used for secure key exchange between parties in the system
 */
export interface Lockbox extends Identifiable {
    /** 
     * Ephemeral keypair used for one-time encryption of this lockbox
     * A new keypair is generated for each lockbox to ensure forward secrecy
     */
    encryptionKey: {
        type: 'EPHEMERAL'
        publicKey: Base58
    }

    /** 
     * Manifest identifying the intended recipient of this lockbox
     * Contains metadata and public key of the recipient who can decrypt the contents
     */
    recipient: KeyManifest

    /** 
     * Manifest describing the encrypted keys contained within this lockbox
     * Helps verify the contents without decryption
     */
    contents: KeyManifest

    /** 
     * The actual encrypted key material
     * Can only be decrypted by the recipient using their private key
     */
    encryptedPayload: Uint8Array
}