import { deserialize, field } from '@dao-xyz/borsh';

export interface IEncryptedAction {
    hash: string;
    prevHash: string;
    recipientPublicKey: string;
    senderPublicKey: string;
    encryptedBody: Uint8Array;
}

export class EncryptedAction implements IEncryptedAction
{
    @field({ type: 'string' })
    hash: string;

    @field({ type: 'string' })
    prevHash: string;

    @field({ type: 'string' })
    recipientPublicKey: string;

    @field({ type: 'string' })
    senderPublicKey: string;

    @field({ type: Uint8Array })
    encryptedBody: Uint8Array;

    static decode(bytes: Uint8Array): EncryptedAction
    {
        return deserialize(bytes, EncryptedAction);
    }

    constructor(data: {
        prevHash: string,
        hash: string,
        recipientPublicKey: string,
        senderPublicKey: string,
        encryptedBody: Uint8Array
    })
    {
        this.prevHash           = data.prevHash;
        this.hash               = data.hash;
        this.recipientPublicKey = data.recipientPublicKey;
        this.senderPublicKey    = data.senderPublicKey;
        this.encryptedBody      = data.encryptedBody;
    }
}
