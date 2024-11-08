import { serialize } from "@dao-xyz/borsh"

export class EncodeHelper {
    encode(): Uint8Array {
        return serialize(this)
    }
}