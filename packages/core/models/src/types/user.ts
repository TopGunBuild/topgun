import { type KeysetPublicInfo, type KeysetPrivateInfo } from './keyset'
import { Identifiable } from './utils'

interface UserBase extends Identifiable {
    /** User's display name or email matching the public User type */
    userName: string
}

/**
 * Represents a user in the system with their public information
 */
export interface UserPublicInfo extends UserBase {
    /** 
     * User's public cryptographic keys
     * Contains public keys for encryption and signature verification
     */
    keys: KeysetPublicInfo
}

/**
 * Extends the base User type with private key information
 * Used only for the currently authenticated user's local data
 * Should never be transmitted or stored unencrypted
 */
export interface UserPrivateInfo extends UserBase {
    /** 
     * Complete set of user's cryptographic keys
     * Includes both public and private keys for all operations
     */
    keys: KeysetPrivateInfo
}
