import { 
    Keyring,
    KeysetPrivateInfo,
    KeysetPublicInfo,
    KeyManifest,
    KeyScopeInfo,
} from "@topgunbuild/models";
import { assert } from "@topgunbuild/common";

/**
 * Type guard to check if an object is a complete keyset with both public and private keys
 * @param value - The value to check
 * @returns True if the value is a complete KeysetWithSecrets
 */
export const isCompleteKeyset = (value: unknown): value is KeysetPrivateInfo => {
    if (!value || typeof value !== 'object') {
        return false
    }

    const keyset = value as KeysetPrivateInfo
    return !!(
        keyset?.encryption?.publicKey &&
        keyset?.encryption?.secretKey &&
        keyset?.signature?.publicKey &&
        keyset?.signature?.secretKey
    )
}

/**
 * Type guard to check if an object is a key manifest containing only public key information
 * @param keys - The keyset or manifest to check
 * @returns True if the object is a KeyManifest
 */
export const isKeyManifest = (
    keys: KeysetPublicInfo | KeysetPrivateInfo | KeyManifest
): keys is KeyManifest => {
    if (!keys || typeof keys !== 'object') {
        return false
    }

    return 'publicKey' in keys && !('secretKey' in keys)
}

/**
 * Type guard to determine if a value is a Keyring rather than a KeysetWithSecrets or array of KeysetWithSecrets
 * @param value - The value to check, which could be a Keyring, KeysetWithSecrets, or KeysetWithSecrets[]
 * @returns True if the value is a Keyring object
 */
export const isKeyring = (value: Keyring | KeysetPrivateInfo | KeysetPrivateInfo[]): value is Keyring => {
    // First check if it's not an array
    if (Array.isArray(value)) {
        return false;
    }
    
    // Then verify it's not a complete keyset
    if (isCompleteKeyset(value)) {
        return false;
    }
    
    // At this point, we know it's a non-array object that's not a complete keyset
    return true;
};

/**
 * Extracts the scope properties from a KeyScope object
 * @param scope - The KeyScope object to extract from
 * @returns A new KeyScope object with just the type and name properties
 */
export const getScope = (scope: KeyScopeInfo): KeyScopeInfo => ({
    type: scope.type,
    name: scope.name,
});

/**
 * Checks if two KeyScope objects have matching type and name values
 * @param scopeA - First KeyScope to compare
 * @param scopeB - Second KeyScope to compare
 * @returns True if both scopes match exactly
 */
export const scopesMatch = (scopeA: KeyScopeInfo, scopeB: KeyScopeInfo): boolean => {
    return scopeA.type === scopeB.type && scopeA.name === scopeB.name;
};

/**
 * Asserts that two KeyScope objects match, throwing an error if they don't
 * @param newScope - The new scope being validated
 * @param existingScope - The existing scope to validate against
 * @throws {Error} If the scopes don't match
 */
export const assertScopesMatch = (newScope: KeyScopeInfo, existingScope: KeyScopeInfo): void => {
    assert(
        scopesMatch(newScope, existingScope),
        `The new keys must have the same scope as the old lockbox keys.\n` +
        `New scope: ${JSON.stringify(getScope(newScope))}\n` +
        `Old scope: ${JSON.stringify(getScope(existingScope))}`
    );
};

/**
 * Type guard to determine if options represent a new team creation vs existing team
 * @param options - The options object to check
 * @returns True if the options are for creating a new team
 * @throws {TypeError} If options is null or not an object
 */
// export const isNewTeam = (options: NewOrExistingTeamOptions): options is NewTeamOptions => {
//     // Validate input is a non-null object
//     if (!options || typeof options !== 'object') {
//         throw new TypeError('Options must be a non-null object');
//     }
    
//     // Check for required teamName property
//     return 'teamName' in options && typeof options.teamName === 'string';
// }