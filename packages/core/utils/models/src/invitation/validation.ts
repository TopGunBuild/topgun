import { signatures } from "@topgunbuild/crypto"
import { InvitationPayload, InvitationState, ProofOfInvitationInfo, ValidationResult } from "@topgunbuild/models"

/**
 * Validates whether an invitation can be used at a given time.
 * Checks for revocation status, usage limits, and expiration.
 * 
 * @param invitation - The invitation state to validate
 * @param timeOfUse - Unix timestamp when the invitation is being used
 * @returns ValidationResult indicating if the invitation is valid
 */
export const validateInvitationUsability = (
  invitation: InvitationState, 
  timeOfUse: number
): ValidationResult => {
  const { revoked, maxUses, uses, expiration } = invitation

  // Check if invitation has been revoked
  if (revoked) {
    return createValidationError('Invitation has been revoked')
  }

  // Check usage limits
  if (maxUses > 0 && uses >= maxUses) {
    return createValidationError('Invitation has reached its usage limit')
  }

  // Check expiration
  if (expiration > 0 && timeOfUse > expiration) {
    return createValidationError('Invitation has expired', {
      expiration,
      timeOfUse,
    })
  }

  return { isValid: true }
}

/**
 * Validates a proof of invitation against the invitation state.
 * Verifies that the proof matches the invitation and contains a valid signature.
 * 
 * @param proof - The proof provided by the invitee
 * @param invitation - The invitation state to validate against
 * @returns ValidationResult indicating if the proof is valid
 */
export const validateInvitationProof = (
  proof: ProofOfInvitationInfo, 
  invitation: InvitationState
): ValidationResult => {
  const { $id, signature } = proof

  // Verify invitation ID match
  if ($id !== invitation.$id) {
    return createValidationError('Invitation ID mismatch', { 
      proofId: $id, 
      invitationId: invitation.$id 
    })
  }

  // Verify signature
  const isSignatureValid = signatures.verify({
    payload: new InvitationPayload($id).encode(),
    signature,
    publicKey: invitation.publicKey,
  })

  if (!isSignatureValid) {
    return createValidationError('Invalid signature', { 
      proof, 
      invitation 
    })
  }

  return { isValid: true }
}

/**
 * Creates a validation error result with optional details
 */
const createValidationError = (message: string, details?: unknown): ValidationResult => ({
  isValid: false,
  error: new InvitationValidationError(message, details),
})

/**
 * Custom error class for invitation validation failures
 */
export class InvitationValidationError extends Error {
  type: string;
  public details?: unknown

  constructor(message: string, details?: unknown) {
    super(message)
    this.name = 'InvitationValidationError'
    this.details = details
  }
}
