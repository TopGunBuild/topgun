import { Keyring, KeysetWithSecrets } from '@topgunbuild/types';
import { deserialize } from '@dao-xyz/borsh';
import { assert, toUint8Array } from '@topgunbuild/utils';
import { asymmetric } from '@topgunbuild/crypto';
import { EncryptedAction } from './encrypted-action';
import { createKeyring } from '../keyset/create-keyring';
import { Action } from './actions';
import { hashAction } from './hash';

export const decryptAction = (
    encryptedAction: EncryptedAction,
    keys: Keyring|KeysetWithSecrets|KeysetWithSecrets[],
) =>
{
    const { senderPublicKey, recipientPublicKey, encryptedBody } = encryptedAction;

    const keyring = createKeyring(keys);
    const keyset  = keyring[recipientPublicKey];

    assert(keyset, `Can't decrypt action: don't have the correct keyset`);

    const cipher = toUint8Array(encryptedBody);

    const decryptedAction = asymmetric.decryptBytes({
        cipher,
        recipientSecretKey: keyset.encryption.secretKey,
        senderPublicKey,
    });
    const action          = deserialize<Action>(decryptedAction, Action);

    return {
        hash: hashAction(encryptedBody),
        action
    };
};
