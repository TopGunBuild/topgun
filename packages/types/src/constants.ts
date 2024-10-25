import { KeyScope } from './common';

export const SIGNATURE   = 'SIGNATURE';
export const ENCRYPTION  = 'ENCRYPTION';
export const SYMMETRIC   = 'SYMMETRIC';
export const ACTION_HASH = 'ACTION_HASH';

export const HashPurpose = {
    SIGNATURE,
    ENCRYPTION,
    SYMMETRIC,
    ACTION_HASH,
} as const;

export const ROOT = 'ROOT';

export const EPHEMERAL_SCOPE: KeyScope = {
    type: 'EPHEMERAL',
    name: 'EPHEMERAL',
};

