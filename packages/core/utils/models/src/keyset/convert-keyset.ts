import { Keyset, KeysetWithSecrets } from "@topgunbuild/types";
import { hasSecrets } from "../utils";

/**
 * Converts a KeysetWithSecrets to a public-only Keyset by removing all secret keys.
 * This is useful when sharing key information with other users or systems that
 * should only have access to public keys.
 *
 * @param keyset - The keyset to convert, can be either a KeysetWithSecrets or already public Keyset
 * @returns A Keyset containing only public keys
 * 
 * @example
 * const publicKeyset = convertToPublicKeyset(userKeys);
 * // Result:
 * // {
 * //   type: 'USER',
 * //   name: 'alice',
 * //   generation: 0,
 * //   signature: 'public-key-data',    // Only public key
 * //   encryption: 'public-key-data',   // Only public key
 * // }
 */
export const convertToPublicKeyset = (keyset: KeysetWithSecrets | Keyset): Keyset => {
    // If the keyset doesn't have secrets, it's already public
    if (!hasSecrets(keyset)) {
        return keyset;
    }

    // Extract only the public components
    return {
        type: keyset.type,
        name: keyset.name,
        generation: keyset.generation,
        encryption: keyset.encryption.publicKey,
        signature: keyset.signature.publicKey,
    };
}