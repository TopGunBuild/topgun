import { field } from '@dao-xyz/borsh';
import { Lockbox } from '../types';
import { KeysetImpl } from './keyset';
import { EncryptionKeyImpl } from './encription-key';
import { EncodeHelper } from '../utils/encode-helper';

export class LockboxImpl extends EncodeHelper implements Lockbox
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    encryptionPublicKey: string;

    @field({ type: KeysetImpl })
    recipient: KeysetImpl;

    @field({ type: KeysetImpl })
    contents: KeysetImpl;

    @field({ type: 'string' })
    encryptedPayload: Uint8Array;

    @field({ type: EncryptionKeyImpl })
    encryptionKey: EncryptionKeyImpl;

    constructor(data: { 
        $id: string, 
        encryptionPublicKey: string, 
        recipient: KeysetImpl, 
        contents: KeysetImpl, 
        encryptedPayload: Uint8Array, 
        encryptionKey: EncryptionKeyImpl 
    })
    {
        super();
        this.$id                   = data.$id;
        this.encryptionPublicKey   = data.encryptionPublicKey;
        this.recipient           = data.recipient;
        this.contents            = data.contents;
        this.encryptedPayload    = data.encryptedPayload;
        this.encryptionKey       = data.encryptionKey;
    }
}
