import { Base58KeypairInfo } from "../types";
import { field } from "@dao-xyz/borsh";
import { EncodeHelper } from "../utils/encode-helper";

export class Base58Keypair extends EncodeHelper implements Base58KeypairInfo {
    @field({ type: 'string' })
    publicKey: string;

    @field({ type: 'string' })
    secretKey: string;

    constructor(data: Base58KeypairInfo) {
        super();
        this.publicKey = data.publicKey;
        this.secretKey = data.secretKey;
    }
}