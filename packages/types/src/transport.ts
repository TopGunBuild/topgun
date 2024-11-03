import { Base58 } from "./keyset"
import { UnixTimestamp, Identifiable } from "./utils"

export type EncryptedPayload = {
    /**
     * The body of the link, encrypted asymmetrically with authentication (using libsodium's
     * `crypto_box`) using the author's SK and the team's PK.
     */
    encryptedBody: Uint8Array
  
    /**
     * Public key of the author of the link, at the time of authoring. After decryption, it is up to
     * the application to ensure that this is in fact the public key of the author (`link.body.user`).
     */
    senderPublicKey: Base58
  
    /**
     * The keys used to decrypt a graph can be rotated at any time. We include the public key of the
     * "recipient" (e.g. the team keys at time of authoring) so that we know which generation of keys
     * to use when decrypting.
     */
    recipientPublicKey: Base58
}
    
/**
 * Metadata for transport messages
 * Contains routing and state information
 */
export interface TransportMetadata {
    /** Identifier of the user sending the message */
    userId: string;
    /** Identifier of the team receiving the message */
    teamId: string;
    /** Optional state counter for versioning and conflict resolution */
    state?: bigint;
    /** Timestamp of the message creation */
    timestamp?: UnixTimestamp;
    /** Type of the message */
    messageType?: string;
    /** Version of the message */
    version?: string;
}

/**
 * Generic transport message structure
 * Combines metadata with typed payload body
 * @template T - The type of the message body
 */
export interface TransportPayload<T> extends Identifiable {
    /** Message metadata for routing and state tracking */
    meta: TransportMetadata;
    /** Typed message content */
    body: T;
    /** Optional message signature */
    signature?: Base58;
}

