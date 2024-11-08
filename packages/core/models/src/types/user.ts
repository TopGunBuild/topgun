import { type Keyset, type KeysetWithSecrets } from './keyset'
import { Identifiable } from './utils'

/**
 * Represents a user in the system with their public information
 */
export interface User extends Identifiable {
    
    /** 
     * User's display name or email address
     * Must be unique across the system but is not used as primary key
     * Used for human-readable identification and external system integration
     */
    userName: string

    /** 
     * User's public cryptographic keys
     * Contains public keys for encryption and signature verification
     */
    keys: Keyset
}

/**
 * Extends the base User type with private key information
 * Used only for the currently authenticated user's local data
 * Should never be transmitted or stored unencrypted
 */
export interface UserWithSecrets extends Identifiable {
    /** User's display name or email matching the public User type */
    userName: string

    /** 
     * Complete set of user's cryptographic keys
     * Includes both public and private keys for all operations
     */
    keys: KeysetWithSecrets
}
