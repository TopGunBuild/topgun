import { asymmetric } from "@topgunbuild/crypto";
import { AbstractAction, EncryptedPayloadImpl, TransportPayloadImpl, UserWithSecrets } from "@topgunbuild/models";

/**
 * Parameters required for payload encryption
 */
export type EncryptedPayloadParams = {
    action: AbstractAction;
    user: UserWithSecrets;
    recipientPublicKey: string;
}

/**
 * Encrypts an action payload for secure transmission between sender and recipient
 * using asymmetric encryption.
 * 
 * @param action - The action to be encrypted
 * @param user - The sender's user object containing encryption keys
 * @param recipientPublicKey - Public key of the recipient
 * @param teamId - The team ID
 * @returns Encrypted payload as Uint8Array
 */
export function encryptPayload({
    action,
    user,
    recipientPublicKey,
}: EncryptedPayloadParams): Uint8Array {

    // Extract sender's encryption keys
    const { publicKey: senderPublicKey, secretKey: senderSecretKey } = user.keys.encryption;

     // Create a transport payload with metadata
     const transportPayload = new TransportPayloadImpl({
        body: action,
        timestamp: Date.now()
    });

    // Perform the actual asymmetric encryption
    const encryptedBody = asymmetric.encryptBytes({
        payload: transportPayload.encode(),
        recipientPublicKey,
        senderSecretKey,
    });

    // Create an encrypted payload container with metadata
    const encryptedPayload = new EncryptedPayloadImpl({
        encryptedBody: encryptedBody,
        senderPublicKey: senderPublicKey,
        recipientPublicKey: recipientPublicKey
    });

    return encryptedPayload.encode();
}