import { KeysetPrivateInfo, LockboxInfo, KeysetPublicInfo } from "@topgunbuild/models"
import { assertScopesMatch } from "../utils"
import { createLockbox } from "./create-lockbox"

interface RotateParameters {
  oldLockbox: LockboxInfo
  newContents: KeysetPrivateInfo
  updatedRecipientKeys?: KeysetPublicInfo
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
}: RotateParameters): LockboxInfo => {
  if (!oldLockbox || !newContents) {
    throw new Error('Both oldLockbox and newContents are required for rotation');
  }

  // Create temporary KeyScope objects to validate scope matching
  const oldContentsScope = { type: oldLockbox.contentsScope, name: oldLockbox.contentsScope }
  const newContentsScope = { type: newContents.name, name: newContents.name }
  assertScopesMatch(newContentsScope, oldContentsScope)

  // If new recipient keys provided, validate their scope
  if (updatedRecipientKeys) {
    const oldRecipientScope = { type: oldLockbox.recipientScope, name: oldLockbox.recipientScope }
    const newRecipientScope = { type: updatedRecipientKeys.name, name: updatedRecipientKeys.name }
    assertScopesMatch(newRecipientScope, oldRecipientScope)
  }

  // Increment the generation counter for the new contents
  const rotatedContents: KeysetPrivateInfo = {
    ...newContents,
    generation: (oldLockbox?.generation ?? 0) + 1
  }

  // Use updated recipient keys if provided, otherwise keep existing ones
  const recipientKeys = updatedRecipientKeys ?? {
    scope: oldLockbox.recipientScope,
    publicKey: oldLockbox.recipientPublicKey,
    type: oldLockbox.recipientType,
    name: oldLockbox.recipientName,
    generation: 0
  }

  return createLockbox({ 
    contents: rotatedContents, 
    recipientKeys 
  })
}
  