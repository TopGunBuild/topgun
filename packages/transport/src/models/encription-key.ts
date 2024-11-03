import { field } from "@dao-xyz/borsh";
import { Base58 } from "@topgunbuild/types";

export class EncryptionKeyImpl
{
    @field({ type: 'string' })
    type: 'EPHEMERAL';

    @field({ type: 'string' })
    publicKey: Base58;

    constructor(data: { type: 'EPHEMERAL', publicKey: Base58 })
    {
        this.type     = data.type;
        this.publicKey = data.publicKey;
    }
}