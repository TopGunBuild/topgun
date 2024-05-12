import { randomBytes as _randomBytes } from '@noble/hashes/utils';

export const randomBytes = (bytesLength = 32) => _randomBytes(bytesLength);
