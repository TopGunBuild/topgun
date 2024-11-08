import { randomKey } from "@topgunbuild/crypto-utils"

/** Length of the invitation key in characters */
export const INVITATION_KEY_LENGTH = 16

/**
 * Generates a secure random invitation key.
 * 
 * The key is a base58-encoded string (e.g., '4kgd5mwq5z4fmfwq') that:
 * - Is 16 characters long by default
 * - Uses base58 to avoid confusing characters (like '1' vs 'l')
 * - Is suitable for sharing via messaging apps or verbal communication
 * 
 * @param length - Optional custom length for the key (default: 16)
 * @returns A random base58 string of the specified length
 * @throws Error if length is less than 1
 * 
 * @example
 * const key = generateInvitationKey() // '4kgd5mwq5z4fmfwq'
 */
export const generateInvitationKey = (length = INVITATION_KEY_LENGTH): string => {
  if (length < 1) {
    throw new Error('Invitation key length must be at least 1 character')
  }
  
  return randomKey(length)
} 