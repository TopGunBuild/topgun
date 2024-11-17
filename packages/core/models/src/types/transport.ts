/**
 * Encrypted payload
 */
export interface EncryptedPayload {
    encryptedBody: Uint8Array
    senderPublicKey: string
    recipientPublicKey?: string
}

/**
 * Generic transport message structure
 * Combines metadata with typed payload body
 * @template T - The type of the message body
 */
export interface TransportPayload<T> {
    /** Timestamp of the message creation */
    timestamp: number;
    /** Typed message content */
    body: T;
}

