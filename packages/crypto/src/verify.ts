import { Signature } from './signature';
import { Ed25519PublicKey, verifySignatureEd25519 } from './ed25519';

export function verify(signature: Signature, data: Uint8Array): boolean
{
    if (signature.publicKey instanceof Ed25519PublicKey)
    {
        return verifySignatureEd25519(signature, data);
    }
    // else if (signature.publicKey instanceof Secp256k1PublicKey)
    // {
    //     return verifySignatureSecp256k1(signature, data);
    // }
    return false;
}
