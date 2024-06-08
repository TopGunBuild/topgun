import { MessageHeader } from './message-header';
import { field, option, serialize, deserialize } from '@dao-xyz/borsh';
import { Signature, verify } from '@topgunbuild/crypto';
import { isBoolean } from '@topgunbuild/utils';

export class Message
{
    @field({ type: MessageHeader })
    header: MessageHeader;

    @field({ type: option(Uint8Array) })
    data?: Uint8Array;

    verified: boolean;

    static decode(bytes: Uint8Array): Message
    {
        return deserialize(bytes, Message);
    }

    get id(): Uint8Array
    {
        return this.header.id;
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
