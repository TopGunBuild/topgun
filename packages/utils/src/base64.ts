import { Input } from '@noble/hashes/utils';
import { base64 } from '@scure/base';
import { sha256 as _sha256 } from '@noble/hashes/sha256';

export const sha256Base64 = (bytes: Input): string => base64.encode(_sha256(bytes));
export const toBase64   = (value: Uint8Array): string => base64.encode(value);
export const fromBase64 = (value: string): Uint8Array => base64.decode(value);
