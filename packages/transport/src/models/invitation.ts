import { field, option } from '@dao-xyz/borsh';

export class Invitation
{
    @field({ type: 'string' })
    id: string;

    @field({ type: 'string' })
    publicKey: string;

    @field({ type: 'string' })
    expiration: string;

    @field({ type: 'u8' })
    maxUses: number;

    @field({ type: option('string') })
    userId?: string;

    constructor(data: { id: string, publicKey: string, expiration: string, maxUses: number, userId?: string })
    {
        this.id         = data.id;
        this.publicKey  = data.publicKey;
        this.expiration = data.expiration;
        this.maxUses    = data.maxUses;
        this.userId     = data.userId;
    }
}
