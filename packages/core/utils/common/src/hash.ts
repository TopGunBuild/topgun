import { sha256 as _sha256 } from '@noble/hashes/sha256';
import { bytesToHex, Input } from '@noble/hashes/utils';

export const sha256 = (bytes: Input): Uint8Array => _sha256(bytes);
export const toHexString = (bytes: Uint8Array): string => bytesToHex(bytes);
