import { ed25519 } from '@noble/curves/ed25519';
import { concatBytes } from '@noble/curves/abstract/utils';
import { field } from '@dao-xyz/borsh';
import { Ed25519PublicKey } from './public-key';
import { Ed25519PrivateKey } from './private-key';
import { PreHash } from '../hash';
import { Keypair } from '../keypair';
import { Signature } from '../signature';
import { sign } from './sign';


export class Ed25519Keypair extends Keypair
{
    @field({ type: Ed25519PublicKey })
    publicKey: Ed25519PublicKey;

    @field({ type: Ed25519PrivateKey })
    privateKey: Ed25519PrivateKey;

    _extendedSecretKey: Uint8Array; // length 64

    static create(): Ed25519Keypair
    {
        const privateKey = ed25519.utils.randomPrivateKey();
        const publicKey  = ed25519.getPublicKey(privateKey);

        return new Ed25519Keypair({
            publicKey : new Ed25519PublicKey(publicKey),
            privateKey: new Ed25519PrivateKey(privateKey),
        });
    }

    /**
     * Constructor
     * @param {{publicKey: Ed25519PublicKey, privateKey: Ed25519PrivateKey}} properties
     */
    constructor(properties: {
        publicKey: Ed25519PublicKey;
        privateKey: Ed25519PrivateKey;
    })
    {
        super();
        this.privateKey = properties.privateKey;
        this.publicKey  = properties.publicKey;
    }

    get extendedSecretKey(): Uint8Array
    {
        if (!this._extendedSecretKey)
        {
            this._extendedSecretKey = concatBytes(this.privateKey.data, this.publicKey.data);
        }

        return this._extendedSecretKey;
    }

    sign(
        data: Uint8Array,
        prehash: PreHash = PreHash.NONE,
    ): Promise<Signature>
    {
        return sign(data, this, prehash);
    }

    equals(other: Keypair): boolean
    {
        if (other instanceof Ed25519Keypair)
        {
            return (
                this.publicKey.equals(other.publicKey) &&
                this.privateKey.equals(other.privateKey)
            );
        }
        return false;
    }
}
