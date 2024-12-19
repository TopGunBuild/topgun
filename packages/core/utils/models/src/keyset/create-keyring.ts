import { KeysetPrivateInfo, Keyring } from "@topgunbuild/models"
import { isKeyring, isCompleteKeyset } from "../utils"

export const createKeyring = (keys: Keyring | KeysetPrivateInfo | KeysetPrivateInfo[]): Keyring => {
    // Return existing keyring if already in correct format
    if (isKeyring(keys)) return keys
  
    // Normalize input to array of keysets
    const keysetArray = isCompleteKeyset(keys) ? [keys] : keys
  
    // Transform array into a map where each keyset is indexed by its public key
    return keysetArray.reduce<Keyring>(
      (acc, keyset) => ({
        ...acc,
        [keyset.encryption.publicKey]: keyset
      }),
      {}
    )
  }
  