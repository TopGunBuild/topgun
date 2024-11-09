import { LocalContext } from "./context"
import { KeysetWithSecrets, Keyring } from "./keyset"
import { Identifiable } from "./utils"

/** Properties required when creating a new team */
export type NewTeamOptions = {
    /** The team's human-facing name */
    teamName: string
  
    /** The team keys need to be provided for encryption and decryption. It's up to the application to persist these somewhere.  */
    teamKeys: KeysetWithSecrets
  }
  
  /** Properties required when rehydrating from an existing graph  */
  export type ExistingTeamOptions = {
    /** Can be serialized or not. */
    source: Uint8Array|any
  
    /** The team keys need to be provided for encryption and decryption. It's up to the application to persist these somewhere.  */
    teamKeyring: Keyring
  }
  
  export type NewOrExistingTeamOptions = NewTeamOptions | ExistingTeamOptions
  
  /** Options passed to the `Team` constructor */
  export type TeamOptions = NewOrExistingTeamOptions & {
    /** A seed for generating keys. This is typically only used for testing, to ensure predictable data. */
    seed?: string
  
    /** Object containing the current user and device (and optionally information about the client & version). */
    context: LocalContext
}

export interface Team extends Identifiable {
    /** The team's human-facing name */
    teamName: string
}