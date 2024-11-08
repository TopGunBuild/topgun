import { UserWithSecrets, User } from "@topgunbuild/types"
import { convertToPublicKeyset } from "../keyset/convert-keyset"

/**
 * Converts a user object (with or without secrets) to a public user object
 * @param user - The user object to convert
 * @returns User object with only public information
 * @throws Error if required user properties are missing
 */
export const convertToPublicUser = (user: User | UserWithSecrets): User => {
    if (!user?.$id || !user?.userName || !user?.keys) {
      throw new Error('Invalid user object: missing required properties')
    }

    const { $id, userName } = user
    return {
      $id,
      userName,
      keys: convertToPublicKeyset(user.keys),
    }
  }