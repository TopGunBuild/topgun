import { field, option } from '@dao-xyz/borsh';
import { InvitationInfo } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

export class Invitation extends EncodeHelper implements InvitationInfo
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    publicKey: string;

    @field({ type: 'f64' })
    expiration: number;

    @field({ type: 'u8' })
    maxUses: number;

    @field({ type: option('string') })
    userId?: string;

    constructor(data: { $id: string, publicKey: string, expiration: number, maxUses: number, userId?: string })
    {
        super();
        this.$id        = data.$id;
        this.publicKey  = data.publicKey;
        this.expiration = data.expiration;
        this.maxUses    = data.maxUses;
        this.userId     = data.userId;
    }
}
