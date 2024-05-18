import { KeySize } from './constants';
import { baseDecode } from '@topgunbuild/utils';

export function decodedPublicKeyFromString(encodedKey: string, size: KeySize): Uint8Array
{
    const parts = encodedKey.split(':');
    let publicKey: string;
    if (parts.length === 1)
    {
        publicKey = parts[0];
    }
    else if (parts.length === 2)
    {
        publicKey = parts[1];
    }
    else
    {
        throw new Error('Invalid encoded key format, must be <curve>:<encoded key>');
    }
    const decodedPublicKey = baseDecode(publicKey);
    if (decodedPublicKey.length !== size)
    {
        throw new Error(`Invalid public key size (${decodedPublicKey.length}), must be ${size}`);
    }

    return decodedPublicKey;
}
