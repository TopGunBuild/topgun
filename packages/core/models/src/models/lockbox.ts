import { field } from '@dao-xyz/borsh';
import { LockboxInfo } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

export class Lockbox extends EncodeHelper implements LockboxInfo {
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    encryptionKeyScope: string;

    @field({ type: 'string' })
    encryptionKeyPublicKey: string;

    @field({ type: 'string' })
    recipientScope: string;

    @field({ type: 'string' })
    recipientPublicKey: string;

    @field({ type: 'string' })
    recipientType: string;

    @field({ type: 'string' })
    recipientName: string;

    @field({ type: 'u32' })
    generation: number;

    @field({ type: 'string' })
    contentsScope: string;

    @field({ type: 'string' })
    contentsPublicKey: string;

    @field({ type: 'string' })
    contentsType: string;

    @field({ type: 'string' })
    contentsName: string;

    @field({ type: 'string' })
    encryptedPayload: Uint8Array;

    constructor(data: {
        $id: string,
        encryptionKeyScope: string,
        encryptionKeyPublicKey: string,
        recipientScope: string,
        recipientPublicKey: string,
        recipientType: string,
        recipientName: string,
        generation: number,
        contentsScope: string,
        contentsPublicKey: string,
        contentsType: string,
        contentsName: string,
        encryptedPayload: Uint8Array,
        }) {
        super();
        this.$id = data.$id;
        this.encryptionKeyScope = data.encryptionKeyScope;
        this.encryptionKeyPublicKey = data.encryptionKeyPublicKey;
        this.recipientScope = data.recipientScope;
        this.recipientPublicKey = data.recipientPublicKey;
        this.recipientType = data.recipientType;
        this.recipientName = data.recipientName;
        this.generation = data.generation;
        this.contentsScope = data.contentsScope;
        this.contentsPublicKey = data.contentsPublicKey;
        this.contentsType = data.contentsType;
        this.contentsName = data.contentsName;
        this.encryptedPayload = data.encryptedPayload;
    }
}
