import { randomBytes } from '@noble/hashes/utils';
import { toHexString } from './hash';

export function randomId(bytesLength = 16): string
{
    return toHexString(randomBytes(bytesLength));
}
