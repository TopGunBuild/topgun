import { Keyring, KeysetWithSecrets } from '@topgunbuild/types';
import { isKeyring, isKeyset } from './utils';

export const createKeyring = (keys: Keyring|KeysetWithSecrets|KeysetWithSecrets[]): Keyring =>
{
    // if it's already a keyring, just return it
    if (isKeyring(keys)) return keys;

    // coerce a single keyset into an array of keysets
    if (isKeyset(keys)) keys = [keys];

    // organize into a map of keysets by public key
    return keys.reduce((accum, k: KeysetWithSecrets) => ({ ...accum, [k.encryption.publicKey]: k }), {});
};
