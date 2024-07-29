import { bytesToHex, equalBytes } from '@noble/curves/abstract/utils';
import { PublicKey } from '../keys';
import { KeySize } from '../constants';
import { field, fixedArray } from '@dao-xyz/borsh';


export class Ed25519PublicKey extends PublicKey
{
    @field({ type: fixedArray('u8', 32) })
    readonly data: Uint8Array;

    /**
     * Creates a PublicKey instance from an Uint8Array.
     * @param {Uint8Array} data
     */
    constructor(data: Uint8Array)
    {
        super();
        this.data = data;
        if (data.length !== KeySize.ED25519)
        {
            throw new Error(`Expecting key to have length ${KeySize.ED25519}`);
        }
    }

    toString(): string
    {
        return 'ed25119p/' + bytesToHex(this.data);
    }

    equals(other: PublicKey): boolean
    {
        if (other instanceof Ed25519PublicKey)
        {
            return equalBytes(this.data, other.data);
        }
        return false;
    }
}
