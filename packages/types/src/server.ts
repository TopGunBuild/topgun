import { type Keyset, type KeysetWithSecrets } from './keyset'
import { Identifiable } from './utils'

/**
 * Represents a server with its complete cryptographic information
 * Includes private keys and should only be stored securely
 */
export interface ServerWithSecrets extends Identifiable {
    /** Server's hostname and optional port */
    host: Host
    
    /** Complete set of server cryptographic keys (public + private) */
    keys: KeysetWithSecrets
}

/**
 * Represents a server's public information
 * Safe to transmit and store in distributed systems
 */
export interface Server extends Identifiable {
    /** Server's hostname and optional port */
    host: Host
    
    /** Public cryptographic keys for the server */
    keys: Keyset
}

/** 
 * Represents a server's hostname, optionally including a port number
 * Examples:
 * - example.com
 * - localhost:8080
 * - 188.26.221.135:443
 */
export type Host = string
