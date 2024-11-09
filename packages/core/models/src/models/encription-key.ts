import { field } from "@dao-xyz/borsh";
import { EncodeHelper } from "../utils/encode-helper";

export class EncryptionKeyImpl extends EncodeHelper
{
    @field({ type: 'string' })
    type: 'EPHEMERAL';

    @field({ type: 'string' })
    publicKey: string;

    constructor(data: { type: 'EPHEMERAL', publicKey: string })
    {
        super();
        this.type     = data.type;
        this.publicKey = data.publicKey;
    }
}