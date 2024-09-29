import { Keyring, Keyset, KeysetWithSecrets } from '@topgunbuild/types';

/** Keyset vs KeysetWithSecrets  */
export const hasSecrets = (keys: Keyset | KeysetWithSecrets): keys is KeysetWithSecrets =>
    keys.encryption.hasOwnProperty('secretKey') &&
    keys.signature.hasOwnProperty('secretKey') &&
    'secretKey' in keys

/** KeysetWithSecrets vs anything else */
export const isKeyset = (
    k: Record<string, unknown> | Array<Record<string, unknown>>
): k is KeysetWithSecrets =>
    k !== undefined && //
    'secretKey' in k &&
    'encryption' in k &&
    'signature' in k

/** Type guard: Keyring vs KeysetWithSecrets  */
export const isKeyring = (k: Keyring | KeysetWithSecrets | KeysetWithSecrets[]): k is Keyring =>
    !Array.isArray(k) && !isKeyset(k)
