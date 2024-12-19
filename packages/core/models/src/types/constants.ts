import { ValidationResult } from "./validator"

import { KeyScopeInfo, KeyType } from "./keyset"

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

export const ADMIN = 'admin'
export const ROOT = 'ROOT'
export const MERGE = 'MERGE'
export const VALID = { isValid: true } as ValidationResult

export const TEAM_SCOPE = { type: KeyType.TEAM, name: KeyType.TEAM } as KeyScopeInfo
export const ADMIN_SCOPE = { type: KeyType.ROLE, name: ADMIN } as KeyScopeInfo
export const EPHEMERAL_SCOPE = {
  type: KeyType.EPHEMERAL,
  name: KeyType.EPHEMERAL,
} as KeyScopeInfo