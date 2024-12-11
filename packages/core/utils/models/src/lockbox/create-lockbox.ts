import { 
  Lockbox, 
  KeysetWithSecrets, 
  EPHEMERAL_SCOPE, 
  KeyManifest, 
  Keyset,
  KeysetWithSecretsImpl
} from "@topgunbuild/models"
import { asymmetric } from "@topgunbuild/crypto"
import { convertToPublicKeyset } from "../keyset/convert-keyset"
import { isKeyManifest } from "../utils"
import { randomId } from "@topgunbuild/common"
import { EncryptionError, ValidationError } from "../errors"

export interface CreateLockboxParams {
  /** The keyset to be encrypted in the lockbox */
  contents: KeysetWithSecrets
  /** The recipient's keys used for encryption */
  recipientKeys: KeysetWithSecrets | Keyset | KeyManifest
}

/**
 * Creates an encrypted lockbox containing a keyset that can only be opened 
 * by the recipient's private key.
 * 
 * @param params - Parameters for creating the lockbox
 * @returns Lockbox containing the encrypted keyset
 * @throws ValidationError if inputs are invalid
 * @throws EncryptionError if encryption fails
 */
export const createLockbox = ({
  contents,
  recipientKeys,
}: CreateLockboxParams): Lockbox => {
  // Validate inputs
  if (!contents?.encryption?.publicKey) {
    throw new ValidationError('Invalid contents: missing required keys')
  }
  if (!recipientKeys) {
    throw new ValidationError('Recipient keys are required')
  }

  try {
    // Redact any sensitive information from keys
    const sanitizedRecipientKeys = sanitizeRecipientKeys(recipientKeys)
    const publicContents = convertToPublicKeyset(contents)

    // Generate ephemeral encryption keys for this lockbox
    const ephemeralKeys = asymmetric.keyPair()
    const recipientPublicKey = extractRecipientPublicKey(sanitizedRecipientKeys)

    // Prepare and encrypt the payload
    const payload = new KeysetWithSecretsImpl(contents)
    const encryptedPayload = encryptPayload(payload, recipientPublicKey, ephemeralKeys.secretKey)

    return {
      $id: randomId(32),
      encryptionKeyScope: EPHEMERAL_SCOPE.type,
      encryptionKeyPublicKey: ephemeralKeys.publicKey,
      recipientScope: sanitizedRecipientKeys.type,
      recipientType: sanitizedRecipientKeys.type,
      recipientName: sanitizedRecipientKeys.name || '',
      generation: 0,
      recipientPublicKey: recipientPublicKey,
      contentsScope: publicContents.type,
      contentsType: publicContents.type,
      contentsName: publicContents.name || '',
      contentsPublicKey: contents.encryption.publicKey,
      encryptedPayload,
    }
  } catch (error) {
    if (error instanceof ValidationError) {
      throw error
    }
    throw new EncryptionError(`Failed to create lockbox: ${error['message']}`)
  }
}

/**
 * Removes any private keys from the recipient's keyset
 */
const sanitizeRecipientKeys = (
  keys: KeysetWithSecrets | Keyset | KeyManifest
): Keyset | KeyManifest => {
  return isKeyManifest(keys) ? keys : convertToPublicKeyset(keys)
}

/**
 * Extracts the public key used for encryption from recipient's keys
 */
const extractRecipientPublicKey = (keys: Keyset | KeyManifest) => {
  return isKeyManifest(keys) ? keys.publicKey : keys.encryption
}

/**
 * Encrypts the payload using the recipient's public key
 */
const encryptPayload = (
  payload: KeysetWithSecretsImpl,
  recipientPublicKey: string,
  senderSecretKey: string
) => {
  return asymmetric.encryptBytes({
    payload: payload.encode(),
    recipientPublicKey,
    senderSecretKey,
  })
}
