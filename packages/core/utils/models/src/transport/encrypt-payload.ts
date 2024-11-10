import { randomId } from "@topgunbuild/common";
import { asymmetric } from "@topgunbuild/crypto";
import { AbstractAction, EncryptedPayloadImpl, TransportPayloadImpl, UserWithSecrets } from "@topgunbuild/models";
import { bigintTime } from "@topgunbuild/time";

/**
 * Parameters required for payload encryption
 */
export type EncryptedPayloadParams = {
    action: AbstractAction;
    user: UserWithSecrets;
    recipientPublicKey: string;
    teamId: string;
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
    teamId,
}: EncryptedPayloadParams): {id: string, payload: Uint8Array} {

    // Extract sender's encryption keys
    const { publicKey: senderPublicKey, secretKey: senderSecretKey } = user.keys.encryption;

    // Create a transport payload with metadata
    const transportPayload = new TransportPayloadImpl({
        $id: randomId(32),
        userId: user.$id,
        teamId: teamId,
        state: bigintTime(),
        body: action,
        timestamp: Date.now()
    });

    // Create an encrypted payload container with metadata
    const encryptedPayload = new EncryptedPayloadImpl({
        encryptedBody: transportPayload.encode(),
        senderPublicKey: senderPublicKey,
        recipientPublicKey: recipientPublicKey
    });

    // Perform the actual asymmetric encryption
    const encryptedBody = asymmetric.encryptBytes({
        payload: encryptedPayload.encode(),
        recipientPublicKey,
        senderSecretKey,
    });

    return {
        id: transportPayload.$id,
        payload: encryptedBody
    };
}