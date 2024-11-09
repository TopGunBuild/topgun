import { Base58Keypair } from "../types";
import { field } from "@dao-xyz/borsh";
import { EncodeHelper } from "../utils/encode-helper";

export class Base58KeypairImpl extends EncodeHelper implements Base58Keypair {
    @field({ type: 'string' })
    publicKey: string;

    @field({ type: 'string' })
    secretKey: string;

    constructor(data: Base58Keypair) {
        super();
        this.publicKey = data.publicKey;
        this.secretKey = data.secretKey;
    }
}