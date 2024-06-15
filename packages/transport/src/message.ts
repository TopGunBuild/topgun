import { MessageHeader } from './message-header';
import { field, option, serialize, deserialize } from '@dao-xyz/borsh';
import { Signature, verify } from '@topgunbuild/crypto';
import { isBoolean, sha256Base64 } from '@topgunbuild/utils';

export class Message
{
    @field({ type: MessageHeader })
    header: MessageHeader;

    @field({ type: option(Uint8Array) })
    data?: Uint8Array;

    verified: boolean;

    private _idString: string;
    private _replyToIdString: string;

    get idString(): string
    {
        if (!this._idString)
        {
            this._idString = sha256Base64(this.header.id);
        }
        return this._idString;
    }

    get replyToIdString(): string
    {
        if (!this._replyToIdString && this.header.replyToId)
        {
            this._replyToIdString = sha256Base64(this.header.replyToId);
        }
        return this._replyToIdString;
    }

    static decode(bytes: Uint8Array): Message
    {
        return deserialize(bytes, Message);
    }

    constructor(properties: { header: MessageHeader; data?: Uint8Array })
    {
        this.data   = properties.data;
        this.header = properties.header;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }

    async sign(
        signer: (bytes: Uint8Array) => Promise<Signature>,
    ): Promise<this>
    {
        const obj             = this;
        const signatures      = obj.header.signatures;
        obj.header.signatures = [];
        const bytes           = serialize(obj);
        obj.header.signatures = signatures;

        const signature       = await signer(bytes);
        obj.header.signatures = signatures ? [...signatures, signature] : [signature];
        return obj;
    }

    async verify(expectSignatures: boolean): Promise<boolean>
    {
        if (!isBoolean(this.verified))
        {
            this.verified = this.header.verify() && (await this.verifySignatures(expectSignatures));
        }

        return this.verified;
    }

    private async verifySignatures(expectSignatures: boolean): Promise<boolean>
    {
        const message    = this;
        const signatures = message.header.signatures;
        if (signatures.length === 0)
        {
            return !expectSignatures;
        }
        message.header.signatures = undefined;
        const bytes               = serialize(message);
        message.header.signatures = signatures;

        for (const signature of signatures)
        {
            if (!verify(signature, bytes))
            {
                return false;
            }
        }
        return true;
    }
}
