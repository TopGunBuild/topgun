import { base58 } from '@scure/base';
import { randomBytes } from '@noble/hashes/utils';

/** Generate an unpredictable key of the specified length (default is 16 characters) and encode it as a base58 string. */
export const randomKey = (length = 16) =>
  // We create a longer key than necessary to have enough base58 characters for truncation to the desired length.
  base58.encode(randomBytes(length * 3)).slice(0, length)
