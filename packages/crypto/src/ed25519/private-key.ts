import { PrivateKey } from '../keys';
import { KeySize } from '../constants';
import { bytesToHex, equalBytes } from '@noble/curves/abstract/utils';
import { field, fixedArray } from '@dao-xyz/borsh';


export class Ed25519PrivateKey extends PrivateKey
{
    @field({ type: fixedArray('u8', 32) })
    readonly data: Uint8Array;

    /**
     * Creates a PrivateKey instance from an Uint8Array.
     * @param {Uint8Array} data
     */
    constructor(data: Uint8Array)
    {
        super();

        if (data.length !== KeySize.ED25519)
        {
            throw new Error(`Expecting key to have length ${KeySize.ED25519}`);
        }

        this.data = data;
    }

    toString(): string
    {
        return 'ed25119s/' + bytesToHex(this.data);
    }

    equals(other: Ed25519PrivateKey): boolean
    {
        if (other instanceof Ed25519PrivateKey)
        {
            return equalBytes(this.data, other.data);
        }
        return false;
    }
}
