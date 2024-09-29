import { hash as createHash } from '@topgunbuild/crypto';
import { ACTION_HASH } from './constants';

export const hashAction = (body: Uint8Array) => createHash(ACTION_HASH, body);
