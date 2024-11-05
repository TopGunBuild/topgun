import { field } from "@dao-xyz/borsh";
import { Base58, Base58Keypair, KeysetWithSecrets } from "@topgunbuild/types";
import { EncodeHelper } from "../utils/encode-helper";
import { Base58KeypairImpl } from "./base58-keypair";

export class KeysetWithSecretsImpl extends EncodeHelper implements KeysetWithSecrets {
    @field({ type: 'string' })
    type: string;

    @field({ type: 'string' })
    name: string;

    @field({ type: 'u32' })
    generation: number;

    @field({ type: 'string' })
    secretKey: Base58;

    @field({ type: Base58KeypairImpl })
    encryption: Base58KeypairImpl;

    @field({ type: Base58KeypairImpl })
    signature: Base58KeypairImpl;

    constructor(data: {
        type: string,
        name: string,
        generation: number,
        secretKey: Base58,
        encryption: Base58Keypair,
        signature: Base58Keypair
    }) {
        super();
        this.type = data.type;
        this.name = data.name;
        this.generation = data.generation || 1;
        this.secretKey = data.secretKey;
        this.encryption = new Base58KeypairImpl(data.encryption);
        this.signature = new Base58KeypairImpl(data.signature);
    }
}