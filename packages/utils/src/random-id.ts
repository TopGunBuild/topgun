import { toHexString } from './hash';
import { randomBytes } from './random-bytes';

export function randomId(): string
{
    return toHexString(randomBytes(16));
}
