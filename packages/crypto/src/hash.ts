import { sha256 } from '@topgunbuild/utils';

export enum PreHash {
    NONE,
    SHA_256,
    // ETH_KECCAK_256
}

export const createHash = (data: Uint8Array, preHash: PreHash): Uint8Array =>
{
    if (preHash === PreHash.NONE)
    {
        return data;
    }
    if (preHash === PreHash.SHA_256)
    {
        return sha256(data);
    }
    // if (prehash === PreHash.ETH_KECCAK_256)
    // {
    //     return ethKeccak256Hash(data);
    // }

    throw new Error('Unsupported');
};

