import { field } from '@dao-xyz/borsh';
import { Base58, Lockbox } from '@topgunbuild/types';
import { KeysetImpl } from './keyset';
import { EncryptionKeyImpl } from './encription-key';

export class LockboxImpl implements Lockbox
{
    @field({ type: 'string' })
    $id: Base58;

    @field({ type: 'string' })
    encryptionPublicKey: Base58;

    @field({ type: KeysetImpl })
    recipient: KeysetImpl;

    @field({ type: KeysetImpl })
    contents: KeysetImpl;

    @field({ type: 'string' })
    encryptedPayload: Uint8Array;

    @field({ type: EncryptionKeyImpl })
    encryptionKey: EncryptionKeyImpl;

    constructor(data: { 
        $id: Base58, 
        encryptionPublicKey: Base58, 
        recipient: KeysetImpl, 
        contents: KeysetImpl, 
        encryptedPayload: Uint8Array, 
        encryptionKey: EncryptionKeyImpl 
    })
    {
        this.$id                   = data.$id;
        this.encryptionPublicKey   = data.encryptionPublicKey;
        this.recipient           = data.recipient;
        this.contents            = data.contents;
        this.encryptedPayload    = data.encryptedPayload;
        this.encryptionKey       = data.encryptionKey;
    }
}
