import { Keyset, KeysetWithSecrets } from "@topgunbuild/types"

/**
 * Type guard to check if a Keyset includes secret keys
 * @param keys - The keyset to check
 * @returns True if the keyset contains all required secret keys
 */
export const hasSecrets = (keys: Keyset | KeysetWithSecrets): keys is KeysetWithSecrets => {
    if (!keys) return false;
    
    return (
        'encryption' in keys &&
        'signature' in keys &&
        'secretKey' in keys &&
        keys.encryption?.hasOwnProperty('secretKey') &&
        keys.signature?.hasOwnProperty('secretKey')
    );
}

/**
 * Type guard to validate if an unknown value is a valid KeysetWithSecrets
 * @param value - The value to check
 * @returns True if the value matches KeysetWithSecrets structure
 */
export const isKeyset = (value: unknown): value is KeysetWithSecrets => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return false;
    }

    const requiredProperties = ['secretKey', 'encryption', 'signature'];
    return requiredProperties.every(prop => prop in value);
}