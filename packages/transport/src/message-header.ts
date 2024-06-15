import { field, fixedArray, option, vec } from '@dao-xyz/borsh';
import { Signature } from '@topgunbuild/crypto';
import { randomBytes, toArray } from '@topgunbuild/utils';

const WEEK_MS = 7 * 24 * 60 * 60 + 1000;

export class MessageHeader
{
    @field({ type: fixedArray('u8', 32) })
    id: Uint8Array;

    @field({ type: option(fixedArray('u8', 32)) })
    replyToId?: Uint8Array;

    @field({ type: vec('string') })
    origin: string[];

    @field({ type: vec(Signature, 'u8') })
    signatures: Signature[];

    @field({ type: 'u64' })
    expires: bigint;

    @field({ type: 'u64' })
    timestamp: bigint;

    constructor(properties: {
        id?: Uint8Array;
        expires?: number;
        origin?: string[];
        replyToId?: Uint8Array
    })
    {
        this.id         = properties?.id || randomBytes(32);
        this.expires    = BigInt(properties?.expires || +new Date() + WEEK_MS);
        this.timestamp  = BigInt(+new Date());
        this.signatures = [];
        this.origin     = toArray(properties?.origin);
        this.replyToId  = properties?.replyToId;
    }

    verify(): boolean
    {
        return this.expires >= +new Date();
    }
}
