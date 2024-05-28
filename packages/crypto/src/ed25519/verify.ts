import { ed25519 } from '@noble/curves/ed25519';
import { Ed25519PublicKey } from './public-key';
import { Signature } from '../signature';
import { createHash } from '../hash';

export const verifySignatureEd25519 = async (signature: Signature, data: Uint8Array): Promise<boolean> =>
{
    let res = false;
    try
    {
        const hashedData = createHash(data, signature.preHash);
        res              = ed25519.verify(
            signature.signature,
            hashedData,
            (signature.publicKey as Ed25519PublicKey).data,
        );
    }
    catch (error)
    {
        return false;
    }
    return res;
};
