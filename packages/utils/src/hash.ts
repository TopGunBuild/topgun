import { sha256 as _sha256 } from '@noble/hashes/sha256';
import { Input } from '@noble/hashes/utils';

export const sha256 = (bytes: Input) => _sha256(bytes);
