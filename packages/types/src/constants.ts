import { ValidationResult } from "./validator"

import { KeyScope } from "./keyset"

export const SIGNATURE = 'SIGNATURE'
export const ENCRYPTION = 'ENCRYPTION'
export const SYMMETRIC = 'SYMMETRIC'
export const LINK_HASH = 'LINK_HASH'
export const INVITATION = 'INVITATION'
export const DEVICE_ID = 'DEVICE_ID'
export const SHARED_KEY = 'SHARED_KEY'

export const HashPurpose = {
  SIGNATURE,
  ENCRYPTION,
  SYMMETRIC,
  LINK_HASH,
  INVITATION,
  DEVICE_ID,
  SHARED_KEY,
} as const

export const ROOT = 'ROOT'
export const MERGE = 'MERGE'
export const VALID = { isValid: true } as ValidationResult

export const EPHEMERAL_SCOPE: KeyScope = {
  type: 'EPHEMERAL',
  name: 'EPHEMERAL',
}