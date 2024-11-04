import { hash, hashPassword } from "@topgunbuild/crypto"
import { Hash, HashPurpose } from "@topgunbuild/types"

/**
 * Generates a unique invitation ID from the seed using scrypt for brute-force protection
 */
export function generateInvitationId(seed: string): Hash {
    const stretchedKey = hashPassword(seed)
    return hash(HashPurpose.INVITATION, stretchedKey).slice(0, 15) as Hash
}