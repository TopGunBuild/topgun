import {
    Optional,
    SIGNATURE,
    ENCRYPTION,
    SYMMETRIC,
    KeyScopeInfo,
    KeysetPrivateInfo,
} from "@topgunbuild/models";
import { asymmetric, hash, hashPassword, randomKey, signatures } from "@topgunbuild/crypto";

export function createKeyset(
    scope: Optional<KeyScopeInfo, 'name'>,
    seed: string = randomKey()
): KeysetPrivateInfo {
    const { type, name = type } = scope;
    
    // Ensure required parameters are present
    if (!type) {
        throw new Error('Scope type is required');
    }

    // Create a deterministic seed for key generation
    const stretchedSeed = hashPassword(`${name}:${type}:${seed}`);
    
    // Generate cryptographic keys with consistent length
    const signatureKey = hash(SIGNATURE, stretchedSeed).slice(0, 32);
    const encryptionKey = hash(ENCRYPTION, stretchedSeed).slice(0, 32);
    const symmetricKey = hash(SYMMETRIC, stretchedSeed);

    return {
        type,
        name,
        generation: 0,
        signature: signatures.keyPair(signatureKey),
        encryption: asymmetric.keyPair(encryptionKey),
        secretKey: symmetricKey,
    };
}