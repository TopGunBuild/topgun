import { field } from '@dao-xyz/borsh';
import { Keyset } from '../models/keyset';

export interface ILockbox
{
    encryptionPublicKey: string;
    recipient: Keyset;
    contents: Keyset;
}

export class Lockbox implements ILockbox
{
    @field({ type: 'string' })
    encryptionPublicKey: string;

    @field({ type: Keyset })
    recipient: Keyset;

    @field({ type: Keyset })
    contents: Keyset;

    constructor(data: { encryptionPublicKey: string, recipient: Keyset, contents: Keyset })
    {
        this.encryptionPublicKey = data.encryptionPublicKey;
        this.recipient           = data.recipient;
        this.contents            = data.contents;
    }
}
