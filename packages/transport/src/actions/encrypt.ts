import { bigintTime } from '@topgunbuild/time';
import { asymmetric } from '@topgunbuild/crypto';
import { KeysetWithSecrets, UserWithSecrets } from '@topgunbuild/types';
import { Action, ActionHeader } from './actions';
import { hashAction } from './hash';
import { EncryptedAction } from './encrypted-action';

export const encryptAction = (
    {
        action,
        user,
        context,
        keys,
        teamId,
        prevHash
    }: EncryptActionParams,
): EncryptedAction =>
{
    // the "sender" of this encrypted link is the user authoring the link
    const { publicKey: senderPublicKey, secretKey: senderSecretKey } = user.keys.encryption;

    // the "recipient" of this encrypted link is whoever knows the secret keys - the current Team keys
    const { publicKey: recipientPublicKey } = keys.encryption;

    // Set header with additional information
    action.header = new ActionHeader({
        context,
        teamId,
        userId: user.userId,
        time  : bigintTime(),
    });

    // create encrypted body
    const encryptedBody = asymmetric.encryptBytes({
        senderSecretKey,
        recipientPublicKey,
        payload: action.encode(),
    });

    // hash is calculated over the encrypted body
    const hash = hashAction(encryptedBody);

    return new EncryptedAction({
        encryptedBody,
        senderPublicKey,
        recipientPublicKey,
        hash,
        prevHash
    });
};

export interface EncryptActionParams
{
    /** The action (type & payload) being added to the graph. */
    action: Action;

    /** User object for the author of this action. */
    user: UserWithSecrets;

    /** Any additional context provided by the application. */
    context?: Uint8Array;

    /** Keyset used to encrypt & decrypt the action. */
    keys: KeysetWithSecrets;

    /** Current team id. */
    teamId: string;

    prevHash: string;
}
