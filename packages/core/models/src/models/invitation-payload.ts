import { field } from "@dao-xyz/borsh"
import { EncodeHelper } from "../utils/encode-helper"

export class InvitationPayload extends EncodeHelper {

    @field({ type: 'string' })
    id: string

    constructor(id: string) {
        super();
        this.id = id
    }
}   