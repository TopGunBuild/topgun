import { hash, hashPassword } from "@topgunbuild/crypto"
import { HashPurpose } from "@topgunbuild/models"

/**
 * Generates a unique invitation ID from the seed using scrypt for brute-force protection
 */
export function generateInvitationId(seed: string): string {
    const stretchedKey = hashPassword(seed)
    return hash(HashPurpose.INVITATION, stretchedKey).slice(0, 15)
}