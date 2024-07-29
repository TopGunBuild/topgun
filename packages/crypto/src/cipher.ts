import { field, variant } from '@dao-xyz/borsh';
import type { ICipher } from './types';

@variant(0)
export class Cipher implements ICipher
{
    @field({ type: Uint8Array })
    nonce: Uint8Array;

    @field({ type: Uint8Array })
    message: Uint8Array;

    constructor(props: { nonce: Uint8Array; message: Uint8Array })
    {
        this.nonce   = props.nonce;
        this.message = props.message;
    }
}
