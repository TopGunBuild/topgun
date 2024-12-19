import { field } from "@dao-xyz/borsh";
import { Base58KeypairInfo, KeysetPrivateInfo } from "../types";
import { EncodeHelper } from "../utils/encode-helper";
import { Base58Keypair } from "./base58-keypair";

export class KeysetPrivate extends EncodeHelper implements KeysetPrivateInfo {
    @field({ type: 'string' })
    type: string;

    @field({ type: 'string' })
    name: string;

    @field({ type: 'u32' })
    generation: number;

    @field({ type: 'string' })
    secretKey: string;

    @field({ type: Base58Keypair })
    encryption: Base58Keypair;

    @field({ type: Base58Keypair })
    signature: Base58Keypair;

    constructor(data: {
        type: string,
        name: string,
        generation: number,
        secretKey: string,
        encryption: Base58KeypairInfo,
        signature: Base58KeypairInfo
    }) {
        super();
        this.type = data.type;
        this.name = data.name;
        this.generation = data.generation || 1;
        this.secretKey = data.secretKey;
        this.encryption = new Base58Keypair(data.encryption);
        this.signature = new Base58Keypair(data.signature);
    }
}