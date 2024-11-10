import { Identifiable } from "./utils"

export interface EncryptedPayload {
    encryptedBody: Uint8Array
    senderPublicKey: string
    recipientPublicKey: string
}

/**
 * Generic transport message structure
 * Combines metadata with typed payload body
 * @template T - The type of the message body
 */
export interface TransportPayload<T> extends Identifiable {
    /** Identifier of the user sending the message */
    userId: string;
    /** Identifier of the team receiving the message */
    teamId: string;
    /** Optional state counter for versioning and conflict resolution */
    state: bigint;
    /** Timestamp of the message creation */
    timestamp: number;
    /** Typed message content */
    body: T;
}

