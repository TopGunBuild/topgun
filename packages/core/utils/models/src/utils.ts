import { KeysetWithSecrets, Keyset, KeyManifest, KeyScope } from "../../../models/dist"
import { assert } from "../../common/dist"

/**
 * Type guard to check if an object is a complete keyset with both public and private keys
 * @param value - The value to check
 * @returns True if the value is a complete KeysetWithSecrets
 */
export const isCompleteKeyset = (value: unknown): value is KeysetWithSecrets => {
    if (!value || typeof value !== 'object') {
        return false
    }

    const keyset = value as KeysetWithSecrets
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
    keys: Keyset | KeysetWithSecrets | KeyManifest
): keys is KeyManifest => {
    if (!keys || typeof keys !== 'object') {
        return false
    }

    return 'publicKey' in keys && !('secretKey' in keys)
}

/**
 * Type guard to check if an object has the basic required properties of a keyset
 * @param value - The value to check
 * @returns True if the value has the minimum required keyset properties
 * @deprecated Use isCompleteKeyset instead for more thorough validation
 */
export const isKeyset = (value: unknown): value is KeysetWithSecrets => {
    if (!value || typeof value !== 'object') {
        return false
    }

    const requiredProperties = ['secretKey', 'encryption', 'signature']
    return requiredProperties.every(prop => prop in value)
}

/**
 * Extracts the scope properties from a KeyScope object
 * @param scope - The KeyScope object to extract from
 * @returns A new KeyScope object with just the type and name properties
 */
export const getScope = (scope: KeyScope): KeyScope => ({
    type: scope.type,
    name: scope.name,
});

/**
 * Checks if two KeyScope objects have matching type and name values
 * @param scopeA - First KeyScope to compare
 * @param scopeB - Second KeyScope to compare
 * @returns True if both scopes match exactly
 */
export const scopesMatch = (scopeA: KeyScope, scopeB: KeyScope): boolean => {
    return scopeA.type === scopeB.type && scopeA.name === scopeB.name;
};

/**
 * Asserts that two KeyScope objects match, throwing an error if they don't
 * @param newScope - The new scope being validated
 * @param existingScope - The existing scope to validate against
 * @throws {Error} If the scopes don't match
 */
export const assertScopesMatch = (newScope: KeyScope, existingScope: KeyScope): void => {
    assert(
        scopesMatch(newScope, existingScope),
        `The new keys must have the same scope as the old lockbox keys.\n` +
        `New scope: ${JSON.stringify(getScope(newScope))}\n` +
        `Old scope: ${JSON.stringify(getScope(existingScope))}`
    );
};