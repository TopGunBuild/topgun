import { equalBytes } from '@noble/curves/abstract/utils';
import { PublicKey } from './keys';
import { PreHash } from './hash';

export class Signature
{
    signature: Uint8Array;
    publicKey: PublicKey;
    preHash: PreHash = PreHash.NONE;

    constructor(props: {
        signature: Uint8Array;
        publicKey: PublicKey;
        preHash: PreHash;
    })
    {
        if (props)
        {
            this.signature = props.signature;
            this.publicKey = props.publicKey;
            this.preHash   = props.preHash;
        }
    }

    equals(other: Signature): boolean
    {
        if (!equalBytes(this.signature, other.signature))
        {
            return false;
        }
        return this.preHash === other.preHash;
    }
}
