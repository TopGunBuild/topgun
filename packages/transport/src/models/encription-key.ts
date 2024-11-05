import { field } from "@dao-xyz/borsh";
import { Base58 } from "@topgunbuild/types";
import { EncodeHelper } from "../utils/encode-helper";

export class EncryptionKeyImpl extends EncodeHelper
{
    @field({ type: 'string' })
    type: 'EPHEMERAL';

    @field({ type: 'string' })
    publicKey: Base58;

    constructor(data: { type: 'EPHEMERAL', publicKey: Base58 })
    {
        super();
        this.type     = data.type;
        this.publicKey = data.publicKey;
    }
}