import { Invitation, ValidationError } from "@topgunbuild/models"
import { UnixTimestamp } from "@topgunbuild/models"
import { normalizeInvitationKey } from "./normalize"
import { generateInviteeStarterKeys } from "./generate-starter-keys"
import { generateInvitationId } from "./generate-id"
import { CryptoError } from "../errors"

export interface CreateInvitationParams {
  /** A randomly generated secret to be passed to the invitee via a side channel */
  seed: string
  /** Time when the invitation expires. If 0, the invitation does not expire. */
  expiration?: UnixTimestamp
  /** Number of times the invitation can be used. If 0, unlimited uses. Default: 1 */
  maxUses?: number
  /** (Device invitations only) User ID the device will be associated with */
  userId?: string
}

/**
 * Creates a new team invitation that can be publicly posted on the team's signature chain.
 * Implementation based on Keybase's Seitan Token v2 exchange protocol.
 * 
 * @param params - Parameters for creating the invitation
 * @returns Invitation object containing public information
 * @throws ValidationError if parameters are invalid
 * @throws CryptoError if key generation fails
 */
export const createInvitation = ({
  seed,
  maxUses = 1,
  expiration = 0 as UnixTimestamp,
  userId,
}: CreateInvitationParams): Invitation => {
  // Validate inputs
  if (!seed) {
    throw new ValidationError('Seed is required')
  }
  if (maxUses < 0) {
    throw new ValidationError('maxUses must be non-negative')
  }
  if (expiration < 0) {
    throw new ValidationError('expiration must be non-negative')
  }

  try {
    const normalizedSeed = normalizeInvitationKey(seed)
    const invitationId = generateInvitationId(normalizedSeed)
    const ephemeralKeys = generateInviteeStarterKeys(normalizedSeed)

    return {
      $id: invitationId,
      publicKey: ephemeralKeys.signature.publicKey,
      expiration,
      maxUses,
      userId
    }
  } catch (error) {
    if (error instanceof ValidationError) {
      throw error
    }
    throw new CryptoError(`Failed to create invitation: ${error['message']}`)
  }
}