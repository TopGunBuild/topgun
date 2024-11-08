import { field } from "@dao-xyz/borsh"
import { Hash } from "../types"
import { EncodeHelper } from "../utils/encode-helper"

export class InvitationPayloadImpl extends EncodeHelper {

    @field({ type: 'string' })
    id: Hash

    constructor(id: Hash) {
        super();
        this.id = id
    }
}   