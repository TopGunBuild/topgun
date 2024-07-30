import { field, option } from '@dao-xyz/borsh';

export class Cipher
{
    @field({ type: Uint8Array })
    nonce: Uint8Array;

    @field({ type: Uint8Array })
    message: Uint8Array;

    @field({ type: option('string') })
    senderPublicKey?: string;

    constructor(props: {
        nonce: Uint8Array;
        message: Uint8Array;
        senderPublicKey?: string;
    })
    {
        this.nonce           = props.nonce;
        this.message         = props.message;
        this.senderPublicKey = props.senderPublicKey;
    }
}
