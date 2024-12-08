import { Identifiable } from "./utils"

/**
 * Represents a secure container (lockbox) for transmitting encrypted keys
 * Used for secure key exchange between parties in the system
 */
export interface Lockbox extends Identifiable {
    /** 
     * Type of the ephemeral encryption key scope
     * Always set to 'EPHEMERAL' for one-time encryption
     */
    encryptionKeyScope: string

    /** 
     * Public key of the ephemeral keypair encoded in Base58 format
     * Used for one-time encryption of this lockbox
     */
    encryptionKeyPublicKey: string

    /** 
     * Type of the recipient's key manifest
     */
    recipientType: string;

    /** 
     * Name/identifier of the recipient's key manifest
     */
    recipientName: string;

    /** 
     * Scope/type of the recipient's key
     */
    recipientScope: string;

    /** 
     * Recipient's public key encoded in Base58 format
     */
    recipientPublicKey: string;

    /**
     * Incremental number indicating the recipient key's generation/version
     */
    recipientGeneration: number;

    /** 
     * Type of the contents key manifest
     */
    contentsType: string;

    /** 
     * Name/identifier of the contents key manifest
     */
    contentsName: string;

    /** 
     * Scope/type of the contents
     */
    contentsScope: string;

    /** 
     * Public key of the encrypted contents
     */
    contentsPublicKey: string;

    /**
     * Incremental number indicating the contents key's generation/version
     */
    contentsGeneration: number;

    /** 
     * The actual encrypted key material
     * Can only be decrypted by the recipient using their private key
     */
    encryptedPayload: Uint8Array;
}