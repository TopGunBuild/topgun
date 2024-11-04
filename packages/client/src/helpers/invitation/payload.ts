import { field, serialize } from "@dao-xyz/borsh";
import { Hash } from "@topgunbuild/types";

export class InvitationPayload {

    @field({ type: 'string' })
    id: Hash

    constructor(id: Hash) {
        this.id = id
    }

    encode(): Uint8Array {
        return serialize(this)
    }
}   