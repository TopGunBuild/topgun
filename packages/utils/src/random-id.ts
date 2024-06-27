import { toHexString } from './hash';
import { randomBytes } from './random-bytes';

export function randomId(bytesLength = 16): string
{
    return toHexString(randomBytes(bytesLength));
}
