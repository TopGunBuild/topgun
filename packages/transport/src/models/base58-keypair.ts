import { Base58, Base58Keypair } from "@topgunbuild/types";
import { field } from "@dao-xyz/borsh";
import { EncodeHelper } from "../utils/encode-helper";

export class Base58KeypairImpl extends EncodeHelper implements Base58Keypair {
    @field({ type: 'string' })
    publicKey: Base58;

    @field({ type: 'string' })
    secretKey: Base58;

    constructor(data: Base58Keypair) {
        super();
        this.publicKey = data.publicKey;
        this.secretKey = data.secretKey;
    }
}