import { Base58, ProofOfInvitation, ValidationError } from "@topgunbuild/types"
import { signatures } from "@topgunbuild/crypto"
import { normalizeInvitationKey } from "./normalize"
import { generateInvitationId } from "./generate-id"
import { generateInviteeStarterKeys } from "./generate-starter-keys"
import { CryptoError } from "../errors"
import { InvitationPayloadImpl } from "@topgunbuild/transport"

/**
 * Generates a cryptographic proof that demonstrates knowledge of the invitation secret
 * without revealing the secret itself. This proof can be verified by team admins
 * to approve new members.
 * 
 * The proof consists of:
 * 1. The invitation ID (derived from the secret)
 * 2. A signature of the ID using ephemeral keys (also derived from the secret)
 * 
 * @param invitationSecret - The secret invitation key shared out-of-band
 * @returns ProofOfInvitation containing the ID and signature
 * @throws ValidationError if the invitation secret is invalid
 * @throws CryptoError if signature generation fails
 */
export const generateInvitationProof = (invitationSecret: string): ProofOfInvitation => {
  // Validate input
  if (!invitationSecret?.trim()) {
    throw new ValidationError('Invitation secret is required')
  }

  try {
    // Normalize the secret to remove formatting characters
    const normalizedSecret = normalizeInvitationKey(invitationSecret)
    
    // Generate the invitation ID and ephemeral keys from the secret
    const invitationId = generateInvitationId(normalizedSecret)
    const ephemeralKeys = generateInviteeStarterKeys(normalizedSecret)
    
    // Create and sign the proof payload
    const payload = new InvitationPayloadImpl(invitationId)
    const signature = signatures.sign(
      payload.encode(), 
      ephemeralKeys.signature.secretKey
    ) as Base58

    return { 
      $id: invitationId, 
      signature 
    }
  } catch (error) {
    if (error instanceof ValidationError) {
      throw error
    }
    throw new CryptoError(`Failed to generate invitation proof: ${error['message']}`)
  }
}
