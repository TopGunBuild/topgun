import { type KeysetPublicInfo, type KeysetPrivateInfo } from './keyset'
import { Identifiable } from './utils'

/**
 * Represents a server hostname with optional port
 * @example
 * - "example.com"
 * - "localhost:8080"
 * - "188.26.221.135:443"
 */
export type ServerHost = string;

/**
 * Base interface containing common server information
 */
interface ServerBase extends Identifiable {
    /** Server's hostname and optional port */
    host: ServerHost;

    /** Timestamp when the server was first registered (Unix timestamp in ms) */
    created: number;
}

/**
 * Represents a server with complete cryptographic information.
 * Includes private keys and should only be stored securely.
 */
export interface ServerPrivateInfo extends ServerBase {
    /** Complete set of server cryptographic keys (public + private) */
    keys: KeysetPrivateInfo;
}

/**
 * Represents a server's public information.
 * Safe to transmit and store in distributed systems.
 */
export interface ServerPublicInfo extends ServerBase {
    /** Public cryptographic keys for the server */
    keys: KeysetPublicInfo;
}