import { KeysetWithSecrets, Lockbox, Keyset } from "@topgunbuild/models"
import { assertScopesMatch } from "../utils"
import { createLockbox } from "./create-lockbox"

interface RotateParameters {
  oldLockbox: Lockbox
  newContents: KeysetWithSecrets
  updatedRecipientKeys?: Keyset
}

/**
 * Rotates a lockbox by replacing its keys with new ones while maintaining security constraints.
 * Used when keys need to be replaced (e.g., after a security compromise).
 * 
 * @param params - Parameters for rotation
 * @param params.oldLockbox - The existing lockbox to rotate
 * @param params.newContents - The new keyset to store in the lockbox
 * @param params.updatedRecipientKeys - Optional new recipient keys
 * @returns A new Lockbox instance with updated contents
 * 
 * @example
 * ```typescript
 * const newAdminKeys = createKeyset({ type: ROLE, name: ADMIN });
 * const newAdminLockbox = rotateLockbox({
 *   oldLockbox: adminLockboxForAlice,
 *   newContents: newAdminKeys
 * });
 * ```
 */
export const rotateLockbox = ({
  oldLockbox,
  newContents,
  updatedRecipientKeys,
}: RotateParameters): Lockbox => {
  if (!oldLockbox || !newContents) {
    throw new Error('Both oldLockbox and newContents are required for rotation');
  }

  // Validate scope matching between old and new contents
  assertScopesMatch(newContents, oldLockbox.contents)

  // If new recipient keys provided, validate their scope
  if (updatedRecipientKeys) {
    assertScopesMatch(oldLockbox.recipient, updatedRecipientKeys)
  }

  // Increment the generation counter for the new contents
  const rotatedContents: KeysetWithSecrets = {
    ...newContents,
    generation: oldLockbox.contents.generation + 1
  }

  // Use updated recipient keys if provided, otherwise keep existing ones
  const recipientManifest = updatedRecipientKeys ?? oldLockbox.recipient

  return createLockbox({ 
    contents: rotatedContents, 
    recipientKeys: recipientManifest 
  })
}
  