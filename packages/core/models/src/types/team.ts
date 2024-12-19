import { LocalContext } from "./context"
import { KeysetPrivateInfo, Keyring } from "./keyset"
import { Identifiable } from "./utils"

/** Properties required when creating a new team */
export type NewTeamOptions = {
    /** The team's human-facing name */
    name: string
  
    /** The team keys need to be provided for encryption and decryption. It's up to the application to persist these somewhere.  */
    keys: KeysetPrivateInfo
  }
  
  /** Properties required when rehydrating from an existing graph  */
  export type ExistingTeamOptions = {
    /** The team's id */
    id: string
  
    /** The team keys need to be provided for encryption and decryption. It's up to the application to persist these somewhere.  */
    keyring: Keyring
  }
  
  export type NewOrExistingTeamOptions = NewTeamOptions | ExistingTeamOptions
  
  /** Options passed to the `Team` constructor */
  export type TeamOptions = NewOrExistingTeamOptions & {
    /** A seed for generating keys. This is typically only used for testing, to ensure predictable data. */
    seed?: string
  
    /** Object containing the current user and device (and optionally information about the client & version). */
    context: LocalContext
}

export interface TeamInfo extends Identifiable {
    /** The team's human-facing name */
    name: string;
    description?: string;
}
