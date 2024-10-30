import { field, option } from '@dao-xyz/borsh';
import { Identifiable } from '../common';
import { randomId } from '@topgunbuild/utils';

export interface IInvitation extends Identifiable
{
    publicKey: string;
    expiration: string;
    maxUses: number;
    userId?: string;
}

export class Invitation implements IInvitation
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    publicKey: string;

    @field({ type: 'string' })
    expiration: string;

    @field({ type: 'u8' })
    maxUses: number;

    @field({ type: option('string') })
    userId?: string;

    constructor(data: { $id?: string, publicKey: string, expiration: string, maxUses: number, userId?: string })
    {
        this.$id        = data.$id || randomId(32);
        this.publicKey  = data.publicKey;
        this.expiration = data.expiration;
        this.maxUses    = data.maxUses;
        this.userId     = data.userId;
    }
}
