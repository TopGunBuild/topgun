import { field } from '@dao-xyz/borsh';
import { Lockbox } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

export class LockboxImpl extends EncodeHelper implements Lockbox {
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
    recipientGeneration: number;

    @field({ type: 'string' })
    contentsScope: string;

    @field({ type: 'string' })
    contentsPublicKey: string;

    @field({ type: 'string' })
    contentsType: string;

    @field({ type: 'string' })
    contentsName: string;

    @field({ type: 'u32' })
    contentsGeneration: number;

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
        recipientGeneration: number,
        contentsScope: string,
        contentsPublicKey: string,
        contentsType: string,
        contentsName: string,
        encryptedPayload: Uint8Array,
        contentsGeneration: number
    }) {
        super();
        this.$id = data.$id;
        this.encryptionKeyScope = data.encryptionKeyScope;
        this.encryptionKeyPublicKey = data.encryptionKeyPublicKey;
        this.recipientScope = data.recipientScope;
        this.recipientPublicKey = data.recipientPublicKey;
        this.recipientType = data.recipientType;
        this.recipientName = data.recipientName;
        this.recipientGeneration = data.recipientGeneration;
        this.contentsScope = data.contentsScope;
        this.contentsPublicKey = data.contentsPublicKey;
        this.contentsType = data.contentsType;
        this.contentsName = data.contentsName;
        this.contentsGeneration = data.contentsGeneration;
        this.encryptedPayload = data.encryptedPayload;
    }
}
