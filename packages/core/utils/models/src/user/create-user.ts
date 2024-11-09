import { randomKey } from "@topgunbuild/crypto"
import { UserWithSecrets, KeyType } from "@topgunbuild/models"
import { createKeyset } from "../keyset/create-keyset"
import { randomId } from "@topgunbuild/common"

/**
 * Creates a new local user, with randomly-generated keys.
 * @param userName - The display name for the user
 * @param userId - Optional user ID (randomly generated if not provided)
 * @param seed - Optional seed for key generation (randomly generated if not provided)
 * @returns UserWithSecrets object containing user details and keys
 */
export const createUser = (
    userName: string,
    userId: string = randomId(),
    seed: string = randomKey()
  ): UserWithSecrets => {
    // Input validation
    if (!userName?.trim()) {
      throw new Error('Username is required')
    }

    return {
      userName: userName.trim(),
      $id: userId,
      keys: createKeyset({ type: KeyType.USER, name: userId }, seed),
    }
  }
  