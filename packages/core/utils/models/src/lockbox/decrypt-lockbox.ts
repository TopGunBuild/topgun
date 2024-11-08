import { Lockbox, KeysetWithSecrets, ValidationError } from "@topgunbuild/types"
import { asymmetric } from "@topgunbuild/crypto"
import { DecryptionError } from "../errors"
import { isCompleteKeyset } from "../utils"

/**
 * Decrypts a lockbox containing encrypted keys using the recipient's decryption keys.
 * A lockbox is a secure container that allows sharing encrypted keys between users.
 * 
 * @param lockbox - The encrypted lockbox containing the keys
 * @param recipientKeys - The recipient's keyset used for decryption
 * @returns The decrypted keyset contained in the lockbox
 * @throws ValidationError if inputs are invalid
 * @throws DecryptionError if decryption fails
 */
export const decryptLockbox = (
  lockbox: Lockbox, 
  recipientKeys: KeysetWithSecrets
): KeysetWithSecrets => {
  // Validate inputs
  if (!lockbox?.encryptionKey?.publicKey || !lockbox?.encryptedPayload) {
    throw new ValidationError('Invalid lockbox: missing required properties')
  }
  if (!recipientKeys?.encryption?.secretKey) {
    throw new ValidationError('Invalid recipient keys: missing secret key')
  }

  try {
    // Attempt to decrypt the lockbox payload
    const decryptedBytes = asymmetric.decryptBytes({
      cipher: lockbox.encryptedPayload,
      senderPublicKey: lockbox.encryptionKey.publicKey,
      recipientSecretKey: recipientKeys.encryption.secretKey,
    })

    // Parse the decrypted bytes into a keyset
    const decryptedKeys = decryptedBytes as unknown as KeysetWithSecrets

    // Validate the decrypted keyset
    if (!isCompleteKeyset(decryptedKeys)) {
      throw new DecryptionError('Decrypted content is not a valid keyset')
    }

    return decryptedKeys
  } catch (error) {
    if (error instanceof ValidationError || error instanceof DecryptionError) {
      throw error
    }
    throw new DecryptionError(`Failed to decrypt lockbox: ${error['message']}`)
  }
}

