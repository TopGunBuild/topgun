import { field, option } from '@dao-xyz/borsh';
import { Base58, Invitation, UnixTimestamp } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

export class InvitationImpl extends EncodeHelper implements Invitation
{
    @field({ type: 'string' })
    $id: Base58;

    @field({ type: 'string' })
    publicKey: Base58;

    @field({ type: 'f64' })
    expiration: UnixTimestamp;

    @field({ type: 'u8' })
    maxUses: number;

    @field({ type: option('string') })
    userId?: string;

    constructor(data: { $id: Base58, publicKey: Base58, expiration: UnixTimestamp, maxUses: number, userId?: string })
    {
        super();
        this.$id        = data.$id;
        this.publicKey  = data.publicKey;
        this.expiration = data.expiration;
        this.maxUses    = data.maxUses;
        this.userId     = data.userId;
    }
}
